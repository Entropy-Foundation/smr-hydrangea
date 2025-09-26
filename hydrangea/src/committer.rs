use aptos_executor::{AptosVmExecutor, LocalAccount, TransactionResult};
use aptos_types::transaction::SignedTransaction;
use log::{error, info, warn};
use primary::{Certificate, Header};
use std::collections::HashSet;
use store::Store;
use tokio::sync::mpsc::Receiver;

const PRE_FUNDED_ACCOUNT_SEEDS: &[u64] = &[1, 2, 3, 4];
const INITIAL_ACCOUNT_BALANCE: u64 = 1_000_000_000_000;

pub struct Committer {
    store: Store,
    executor: AptosVmExecutor,
    rx_commit: Receiver<Vec<Certificate>>,
}

impl Committer {
    pub fn spawn(store: Store, rx_commit: Receiver<Vec<Certificate>>) {
        tokio::spawn(async move {
            let executor = match AptosVmExecutor::new() {
                Ok(executor) => executor,
                Err(e) => {
                    error!("Failed to initialize Aptos VM executor: {}", e);
                    return;
                }
            };

            bootstrap_accounts(&executor);

            let mut committer = Self {
                store,
                executor,
                rx_commit,
            };
            committer.run().await;
        });
    }

    async fn run(&mut self) {
        while let Some(certificates) = self.rx_commit.recv().await {
            #[cfg(feature = "benchmark")]
            for certificate in &certificates {
                info!("Committed Header {:?}", certificate.id);
            }

            let mut transactions: Vec<SignedTransaction> = Vec::new();
            for certificate in certificates {
                match self.load_header(&certificate).await {
                    Some(header) => transactions.extend(header.payload),
                    None => warn!(
                        "Missing header for certificate {:?} (round {})",
                        certificate.id, certificate.round
                    ),
                }
            }

            if transactions.is_empty() {
                continue;
            }

            let transactions = deduplicate_transactions(transactions);
            if transactions.is_empty() {
                continue;
            }

            let results = self.executor.execute_block(&transactions);
            log_execution_results(&transactions, &results);
        }
    }

    async fn load_header(&self, certificate: &Certificate) -> Option<Header> {
        let mut store = self.store.clone();
        match store.read(certificate.id.to_vec()).await {
            Ok(Some(bytes)) => match bincode::deserialize::<Header>(&bytes) {
                Ok(header) => Some(header),
                Err(e) => {
                    warn!(
                        "Failed to deserialize header for certificate {:?}: {}",
                        certificate.id, e
                    );
                    None
                }
            },
            Ok(None) => {
                warn!(
                    "No header found in storage for certificate {:?}",
                    certificate.id
                );
                None
            }
            Err(e) => {
                warn!(
                    "Store read failure for certificate {:?}: {}",
                    certificate.id, e
                );
                None
            }
        }
    }
}

fn bootstrap_accounts(executor: &AptosVmExecutor) {
    for seed in PRE_FUNDED_ACCOUNT_SEEDS {
        match LocalAccount::generate(*seed) {
            Ok(account) => {
                executor.bootstrap_account(&account, INITIAL_ACCOUNT_BALANCE);
                info!("Bootstrapped Aptos account {:?}", account.address);
            }
            Err(e) => warn!("Failed to generate deterministic account {}: {}", seed, e),
        }
    }
}

fn log_execution_results(transactions: &[SignedTransaction], results: &[TransactionResult]) {
    for (index, (txn, result)) in transactions.iter().zip(results.iter()).enumerate() {
        let status_display = format!("{:?}", result.status());
        let gas_used = result.gas_used();
        info!(
            "Executed transaction {} ({} BCS bytes): status={}, gas_used={}",
            index,
            serialized_len(txn),
            status_display,
            gas_used
        );
    }
}

fn serialized_len(tx: &SignedTransaction) -> usize {
    bcs::serialized_size(tx).expect("failed to compute serialized transaction size") as usize
}

fn deduplicate_transactions(transactions: Vec<SignedTransaction>) -> Vec<SignedTransaction> {
    let mut seen: HashSet<Vec<u8>> = HashSet::with_capacity(transactions.len());
    let mut unique = Vec::with_capacity(transactions.len());

    for txn in transactions {
        match bcs::to_bytes(&txn) {
            Ok(bytes) => {
                if seen.insert(bytes) {
                    unique.push(txn);
                }
            }
            Err(error) => {
                warn!(
                    "Failed to serialize transaction for deduplication: {}",
                    error
                );
                unique.push(txn);
            }
        }
    }

    unique
}
