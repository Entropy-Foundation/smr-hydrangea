// Copyright(C) Facebook, Inc. and its affiliates.
use crate::batch_maker::Transaction;
use crate::messages::Header;
use crate::primary::Round;
use crypto::{PublicKey, SignatureService};
#[cfg(feature = "benchmark")]
use log::info;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::{sleep, Duration, Instant};

// #[cfg(test)]
// #[path = "tests/proposer_tests.rs"]
// pub mod proposer_tests;

/// The proposer creates new headers and send them to the core for broadcasting and further processing.
pub struct Proposer {
    /// The public key of this primary.
    name: PublicKey,
    /// Service to sign headers.
    signature_service: SignatureService,
    /// The size of the headers' payload.
    header_size: usize,
    /// The maximum delay to wait for batches' digests.
    max_header_delay: u64,
    /// Receives the batches' digests from our workers.
    rx_workers: Receiver<Vec<Transaction>>,
    /// Sends newly created headers to the `Core`.
    tx_core: Sender<Header>,
    /// The current round of the dag.
    round: Round,
    /// Holds the batches' digests waiting to be included in the next header.
    txns: Vec<Transaction>,
    /// Keeps track of the size (in bytes) of batches' digests that we received so far.
    payload_size: usize,
}

impl Proposer {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        name: PublicKey,
        signature_service: SignatureService,
        header_size: usize,
        max_header_delay: u64,
        rx_workers: Receiver<Vec<Transaction>>,
        tx_core: Sender<Header>,
    ) {
        tokio::spawn(async move {
            Self {
                name,
                signature_service,
                header_size,
                max_header_delay,
                rx_workers,
                tx_core,
                round: 1,
                txns: Vec::with_capacity(2 * header_size),
                payload_size: 0,
            }
            .run()
            .await;
        });
    }

    async fn make_header(&mut self) {
        // Make a new header.
        let header = Header::new(
            self.name,
            self.round,
            self.txns.drain(..).collect(),
            &mut self.signature_service,
        )
        .await;

        #[cfg(feature = "benchmark")]
        {
            info!("Created Header {:?}", header.id);
            info!("Header {:?} contains {} B", header.id, self.payload_size);

            // NOTE: This log entry is used to compute performance.
            let tx_ids: Vec<_> = header.payload.iter().filter_map(sample_tx_id).collect();
            for id in tx_ids {
                info!(
                    "Header {:?} contains sample tx {}",
                    header.id,
                    u64::from_be_bytes(id)
                );
            }
        }

        // Send the new header to the `Core` that will broadcast and process it.
        self.tx_core
            .send(header)
            .await
            .expect("Failed to send header");
    }

    // Main loop listening to incoming messages.
    pub async fn run(&mut self) {
        // debug!("Dag starting at round {}", self.round);

        let timer = sleep(Duration::from_millis(self.max_header_delay));
        tokio::pin!(timer);

        loop {
            // Check if we can propose a new header. We propose a new header when one of the following
            // conditions is met:
            // 1. Enough batches' digests;
            // 2. The specified maximum inter-header delay has passed.
            let enough_digests = self.payload_size >= self.header_size;
            let timer_expired = timer.is_elapsed();
            if (timer_expired && self.payload_size > 0) || enough_digests {
                // Make a new header.
                self.make_header().await;
                self.payload_size = 0;

                // Reschedule the timer.
                let deadline = Instant::now() + Duration::from_millis(self.max_header_delay);
                timer.as_mut().reset(deadline);
            }

            tokio::select! {
                Some(transactions) = self.rx_workers.recv() => {
                    self.payload_size +=
                        transactions.iter().map(serialized_len).sum::<usize>();
                    self.txns.extend(transactions);
                }
                () = &mut timer => {
                    // Nothing to do.

                }
            }
        }
    }
}

fn serialized_len(tx: &Transaction) -> usize {
    bcs::serialized_size(tx).expect("failed to compute serialized transaction size") as usize
}

#[cfg(feature = "benchmark")]
fn sample_tx_id(tx: &Transaction) -> Option<[u8; 8]> {
    let bytes = bcs::to_bytes(tx).ok()?;
    if bytes.first().copied() != Some(0u8) || bytes.len() < 9 {
        return None;
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&bytes[1..9]);
    Some(id)
}
