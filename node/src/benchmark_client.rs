// Copyright(C) Facebook, Inc. and its affiliates.
use anyhow::{Context, Result};
use aptos_executor::{transaction_builder::apt_transfer, LocalAccount};
use aptos_types::chain_id::ChainId;
use bytes::Bytes;
use clap::{crate_name, crate_version, App, AppSettings};
use env_logger::Env;
use futures::future::join_all;
use futures::sink::SinkExt as _;
use log::{info, warn};
use std::cmp::max;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use tokio::time::{interval, sleep, Duration, Instant};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[tokio::main]
async fn main() -> Result<()> {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .about("Benchmark client for Narwhal and Tusk.")
        .args_from_usage("<ADDR> 'The network address of the node where to send txs'")
        .args_from_usage("--size=<INT> 'The size of each transaction in bytes'")
        .args_from_usage("--burst=<INT> 'Burst duration (in ms)'")
        .args_from_usage("--rate=<INT> 'The rate (txs/s) at which to send the transactions'")
        .args_from_usage("--nodes=[ADDR]... 'Network addresses that must be reachable before starting the benchmark.'")
        .setting(AppSettings::ArgRequiredElseHelp)
        .get_matches();

    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let target = matches
        .value_of("ADDR")
        .unwrap()
        .parse::<SocketAddr>()
        .context("Invalid socket address format")?;
    let size = matches
        .value_of("size")
        .unwrap()
        .parse::<usize>()
        .context("The size of transactions must be a non-negative integer")?;
    let burst_duration = matches
        .value_of("burst")
        .unwrap()
        .parse::<u64>()
        .context("Burst duration must be a non-negative integer")?;
    let rate = matches
        .value_of("rate")
        .unwrap()
        .parse::<u64>()
        .context("The rate of transactions must be a non-negative integer")?;
    let nodes = matches
        .values_of("nodes")
        .unwrap_or_default()
        .into_iter()
        .map(|x| x.parse::<SocketAddr>())
        .collect::<Result<Vec<_>, _>>()
        .context("Invalid socket address format")?;

    info!("Node address: {}", target);

    // NOTE: This log entry is used to compute performance.
    info!("Requested transaction size: {} B", size);

    // NOTE: This log entry is used to compute performance.
    info!("Transactions rate: {} tx/s", rate);

    let chain_id = ChainId::test();
    let transfer_amount = 1u64;

    let recipient = LocalAccount::generate(2).context("failed to create recipient account")?;
    let mut sample_sender = LocalAccount::generate(1).context("failed to create sample sender")?;
    let sample_tx = apt_transfer(
        &mut sample_sender,
        recipient.address,
        transfer_amount,
        chain_id,
    )
    .context("failed to build sample transaction")?;
    let tx_size_bytes = bcs::to_bytes(&sample_tx)
        .context("failed to serialize sample transaction")?
        .len();

    info!(
        "Aptos transfer transaction size: {} B (serialized)",
        tx_size_bytes
    );

    let sender = LocalAccount::generate(1).context("failed to create sender account")?;

    let mut client = Client {
        target,
        rate,
        nodes,
        burst_duration,
        sender,
        recipient,
        chain_id,
        transfer_amount,
        tx_size_bytes,
    };

    // Wait for all nodes to be online and synchronized.
    client.wait().await;

    // Start the benchmark.
    client.send().await.context("Failed to submit transactions")
}

struct Client {
    target: SocketAddr,
    rate: u64,
    nodes: Vec<SocketAddr>,
    burst_duration: u64,
    sender: LocalAccount,
    recipient: LocalAccount,
    chain_id: ChainId,
    transfer_amount: u64,
    tx_size_bytes: usize,
}

impl Client {
    pub async fn send(&mut self) -> Result<()> {
        const PRECISION: u64 = 20; // Sample precision.
        info!("Burst duration {:?}", self.burst_duration);

        if self.rate == 0 {
            warn!("Transaction rate is zero; no transactions will be sent");
            return Ok(());
        }

        // Connect to the mempool.
        let stream = TcpStream::connect(self.target)
            .await
            .context(format!("failed to connect to {}", self.target))?;

        // Submit all transactions.
        let burst = max(1, self.rate / PRECISION);
        let mut counter: u64 = 0;
        let mut transport = Framed::new(stream, LengthDelimitedCodec::new());
        let interval = interval(Duration::from_millis(self.burst_duration));
        tokio::pin!(interval);

        info!(
            "Start sending transactions (serialized size: {} B)",
            self.tx_size_bytes
        );

        'main: loop {
            interval.as_mut().tick().await;
            let start = Instant::now();

            for i in 0..burst {
                let sequence = self.sender.sequence_number;
                if i == counter % burst {
                    info!(
                        "Sending sample transaction {} (sequence {})",
                        counter, sequence
                    );
                }

                let txn = apt_transfer(
                    &mut self.sender,
                    self.recipient.address,
                    self.transfer_amount,
                    self.chain_id,
                )?;
                let bytes = bcs::to_bytes(&txn)?;
                if let Err(e) = transport.send(Bytes::from(bytes)).await {
                    warn!("Failed to send transaction: {}", e);
                    break 'main;
                }
                counter = counter.wrapping_add(1);
            }

            if start.elapsed().as_millis() > self.burst_duration as u128 {
                warn!("Transaction rate too high for this client");
            }
        }

        Ok(())
    }

    pub async fn wait(&self) {
        // Wait for all nodes to be online.
        info!("Waiting for all nodes to be online...");
        join_all(self.nodes.iter().cloned().map(|address| {
            tokio::spawn(async move {
                while TcpStream::connect(address).await.is_err() {
                    sleep(Duration::from_millis(10)).await;
                }
            })
        }))
        .await;
    }
}
