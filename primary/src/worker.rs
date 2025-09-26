use crate::batch_maker::{Batch, BatchMaker, Transaction};
use async_trait::async_trait;
use bytes::Bytes;
use config::{Committee, Parameters, WorkerId};
use crypto::{Digest, PublicKey};
use log::{info, warn};
use network::{MessageHandler, Receiver, Writer};
use serde::{Deserialize, Serialize};
use std::error::Error;
use tokio::sync::mpsc::{channel, Sender};

// #[cfg(test)]
// #[path = "tests/worker_tests.rs"]
// pub mod worker_tests;

/// The default channel capacity for each channel of the worker.
pub const CHANNEL_CAPACITY: usize = 1_000;

/// The message exchanged between workers.
#[derive(Debug, Serialize, Deserialize)]
pub enum WorkerMessage {
    Batch(Batch),
    BatchRequest(Vec<Digest>, /* origin */ PublicKey),
}

pub struct Worker {
    /// The public key of this authority.
    name: PublicKey,
    /// The id of this worker.
    id: WorkerId,
    /// The committee information.
    committee: Committee,
    /// The configuration parameters.
    parameters: Parameters,
    tx_digests: Sender<Vec<Transaction>>,
}

impl Worker {
    pub fn spawn(
        name: PublicKey,
        id: WorkerId,
        committee: Committee,
        parameters: Parameters,
        tx_digests: Sender<Vec<Transaction>>,
    ) {
        // Define a worker instance.
        let worker = Self {
            name,
            id,
            committee,
            parameters,
            tx_digests,
        };

        // Spawn all worker tasks.
        // let (tx_primary, rx_primary) = channel(CHANNEL_CAPACITY);
        worker.handle_clients_transactions();

        // NOTE: This log entry is used to compute performance.
        info!(
            "Worker {} successfully booted on {}",
            id,
            worker
                .committee
                .worker(&worker.name, &worker.id)
                .expect("Our public key or worker id is not in the committee")
                .transactions
                .ip()
        );
    }

    /// Spawn all tasks responsible to handle clients transactions.
    fn handle_clients_transactions(&self) {
        let (tx_batch_maker, rx_batch_maker) = channel(CHANNEL_CAPACITY);

        // We first receive clients' transactions from the network.
        let mut address = self
            .committee
            .worker(&self.name, &self.id)
            .expect("Our public key or worker id is not in the committee")
            .transactions;
        address.set_ip("0.0.0.0".parse().unwrap());
        Receiver::spawn(
            address,
            /* handler */ TxReceiverHandler { tx_batch_maker },
        );

        // The transactions are sent to the `BatchMaker` that assembles them into batches. It then broadcasts
        // (in a reliable manner) the batches to all other workers that share the same `id` as us. Finally, it
        // gathers the 'cancel handlers' of the messages and send them to the `QuorumWaiter`.
        BatchMaker::spawn(
            self.parameters.batch_size,
            self.parameters.max_batch_delay,
            /* rx_transaction */ rx_batch_maker,
            self.tx_digests.clone(),
        );

        info!(
            "Worker {} listening to client transactions on {}",
            self.id, address
        );
    }
}

/// Defines how the network receiver handles incoming transactions.
#[derive(Clone)]
struct TxReceiverHandler {
    tx_batch_maker: Sender<Transaction>,
}

#[async_trait]
impl MessageHandler for TxReceiverHandler {
    async fn dispatch(&self, _writer: &mut Writer, message: Bytes) -> Result<(), Box<dyn Error>> {
        // Parse the transaction and forward it to the batch maker.
        let txn: Transaction = match bcs::from_bytes(message.as_ref()) {
            Ok(txn) => txn,
            Err(e) => {
                warn!("Failed to decode incoming transaction: {}", e);
                return Ok(());
            }
        };
        self.tx_batch_maker
            .send(txn)
            .await
            .expect("Failed to send transaction");

        // Give the change to schedule other tasks.
        tokio::task::yield_now().await;
        Ok(())
    }
}
