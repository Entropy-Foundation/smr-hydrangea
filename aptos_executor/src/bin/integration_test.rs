use anyhow::{bail, Context, Result};
use aptos_executor::{transaction_builder::apt_transfer, LocalAccount};
use aptos_types::{chain_id::ChainId, transaction::SignedTransaction};
use bytes::Bytes;
use config::{Comm, Import, WorkerId};
use futures::SinkExt;
use std::{
    env,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, task, time::sleep};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const TRANSFER_AMOUNTS: [u64; 3] = [100, 150, 200];
const WORKER_ID: WorkerId = 0;
const DEFAULT_LOCAL_DIR: &str = "scripts/.local";
const EXPECTED_EXECUTED_TXS: usize = 3;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let local_dir = resolve_local_dir();
    let committee_path = local_dir.join("config/committee.json");
    let log_path = resolve_log_path(&local_dir);

    println!("Loading committee from {}", committee_path.display());
    let worker_addresses = load_worker_addresses(&committee_path)?;
    if worker_addresses.is_empty() {
        bail!("no worker transaction addresses found in committee file");
    }

    println!(
        "Discovered {} worker transaction endpoints",
        worker_addresses.len()
    );

    let transactions = build_transfer_sequence()?;
    println!("Submitting transfer sequence to consensus:");
    println!("  1. A sends {} tokens to B", TRANSFER_AMOUNTS[0]);
    println!("  2. B sends {} tokens to C", TRANSFER_AMOUNTS[1]);
    println!("  3. C sends {} tokens to A", TRANSFER_AMOUNTS[2]);

    for (idx, txn) in transactions.iter().enumerate() {
        for addr in &worker_addresses {
            submit_transaction(*addr, txn)
                .await
                .with_context(|| format!("failed to submit txn {} to {}", idx + 1, addr))?;
        }
        println!("  ✓ Submitted transaction {}", idx + 1);
    }

    println!(
        "Waiting for committer log '{}' to report executed transactions...",
        log_path.display()
    );
    task::spawn_blocking(move || {
        wait_for_execution_logs(&log_path, EXPECTED_EXECUTED_TXS, Duration::from_secs(40))
    })
    .await
    .context("log watcher task failed")??;

    println!("All transactions executed via consensus.");
    Ok(())
}

fn resolve_local_dir() -> PathBuf {
    if let Ok(path) = env::var("HYDRANGEA_LOCAL_DIR") {
        return PathBuf::from(path);
    }
    PathBuf::from(DEFAULT_LOCAL_DIR)
}

fn resolve_log_path(local_dir: &Path) -> PathBuf {
    if let Ok(path) = env::var("HYDRANGEA_NODE_LOG") {
        return PathBuf::from(path);
    }
    local_dir.join("logs/node-0.log")
}

fn load_worker_addresses(path: &Path) -> Result<Vec<SocketAddr>> {
    let comm = Comm::import(path.to_str().unwrap())
        .with_context(|| format!("failed to import committee from {}", path.display()))?;
    let mut addresses = Vec::new();
    for authority in comm.authorities.values() {
        if let Some(worker) = authority.workers.get(&WORKER_ID) {
            addresses.push(worker.transactions);
        }
    }
    Ok(addresses)
}

fn build_transfer_sequence() -> Result<Vec<SignedTransaction>> {
    let chain_id = ChainId::test();

    let mut account_a = LocalAccount::generate(1).context("failed to generate account A")?;
    let mut account_b = LocalAccount::generate(2).context("failed to generate account B")?;
    let mut account_c = LocalAccount::generate(3).context("failed to generate account C")?;

    let tx1 = apt_transfer(
        &mut account_a,
        account_b.address,
        TRANSFER_AMOUNTS[0],
        chain_id,
    )
    .context("failed to build A -> B transfer")?;
    let tx2 = apt_transfer(
        &mut account_b,
        account_c.address,
        TRANSFER_AMOUNTS[1],
        chain_id,
    )
    .context("failed to build B -> C transfer")?;
    let tx3 = apt_transfer(
        &mut account_c,
        account_a.address,
        TRANSFER_AMOUNTS[2],
        chain_id,
    )
    .context("failed to build C -> A transfer")?;

    Ok(vec![tx1, tx2, tx3])
}

async fn submit_transaction(addr: SocketAddr, txn: &SignedTransaction) -> Result<()> {
    let payload = Bytes::from(bcs::to_bytes(txn)?);
    let mut attempt: u32 = 0;
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let mut codec = LengthDelimitedCodec::new();
                codec.set_max_frame_length(5 * 1024 * 1024);
                let mut framed = Framed::new(stream, codec);
                framed
                    .send(payload.clone())
                    .await
                    .context("failed to send transaction bytes")?;
                return Ok(());
            }
            Err(error) => {
                if attempt > 20 {
                    return Err(error).context("exhausted retries connecting to worker");
                }
                attempt += 1;
                sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

fn wait_for_execution_logs(path: &Path, expected: usize, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    let mut processed = 0usize;
    let mut offset = 0u64;

    while start.elapsed() <= timeout {
        if let Ok(mut file) = File::open(path) {
            file.seek(SeekFrom::Start(offset))
                .context("failed to seek log file")?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                let bytes = reader.read_line(&mut line).context("failed to read log")?;
                if bytes == 0 {
                    break;
                }
                if line.contains("Executed transaction")
                    && line.to_ascii_uppercase().contains("STATUS=EXECUTED")
                {
                    processed += 1;
                }
            }
            let mut file = reader.into_inner();
            offset = file
                .stream_position()
                .context("failed to get file position")?;
        }

        if processed >= expected {
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(500));
    }

    bail!(
        "timed out after {:?} waiting for {} executed transactions (observed {})",
        timeout,
        expected,
        processed
    );
}
