use anyhow::{bail, Context, Result};
use aptos_executor::scenarios::three_trader::{
    build_three_trader_transactions, resolve_package_dir, wait_for_execution_logs,
    EXPECTED_SCENARIO_TXNS,
};
use aptos_types::{chain_id::ChainId, transaction::SignedTransaction};
use bytes::Bytes;
use config::{Comm, Import, WorkerId};
use futures::SinkExt;
use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{net::TcpStream, task, time::sleep};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const WORKER_ID: WorkerId = 0;
const DEFAULT_LOCAL_DIR: &str = "scripts/.local";
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let local_dir = resolve_local_dir();
    let committee_path = local_dir.join("config/committee.json");
    let log_path = resolve_log_path(&local_dir);
    let package_dir = resolve_package_dir()?;
    let chain_id = ChainId::test();

    println!("Loading committee from {}", committee_path.display());
    let worker_addresses = load_worker_addresses(&committee_path)?;
    if worker_addresses.is_empty() {
        bail!("no worker transaction addresses found in committee file");
    }
    println!(
        "Discovered {} worker transaction endpoints",
        worker_addresses.len()
    );

    println!(
        "Loading simple_market package from {}",
        package_dir.display()
    );
    let scenario = build_three_trader_transactions(&package_dir, chain_id)?;

    println!("Submitting three-trader demo sequence to consensus:");
    for (index, scenario_txn) in scenario.iter().enumerate() {
        for addr in &worker_addresses {
            submit_transaction(*addr, &scenario_txn.txn)
                .await
                .with_context(|| {
                    format!(
                        "failed to submit step {} ({}) to {}",
                        index + 1,
                        scenario_txn.label,
                        addr
                    )
                })?;
        }
        println!("  ✓ Step {}: {}", index + 1, scenario_txn.label);
    }

    println!(
        "Waiting for committer log '{}' to report executed transactions...",
        log_path.display()
    );
    task::spawn_blocking(move || {
        wait_for_execution_logs(&log_path, EXPECTED_SCENARIO_TXNS, Duration::from_secs(60))
    })
    .await
    .context("log watcher task failed")??;

    println!("All three-trader demo transactions executed via consensus.");
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
