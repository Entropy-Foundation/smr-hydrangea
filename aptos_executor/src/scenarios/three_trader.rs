use crate::transaction_builder::{
    cancel_order_by_client_id, create_market, decrease_order_size_by_client_id, mint_trader_funds,
    place_limit_order_with_client_id, publish_package, register_trader, replace_order_by_client_id,
};
use crate::LocalAccount;
use anyhow::{bail, Context, Result};
use aptos_types::{chain_id::ChainId, transaction::SignedTransaction};
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_ALLOW_SELF_MATCHING: bool = false;
pub const DEFAULT_ALLOW_EVENTS_EMISSION: bool = true;
pub const DEFAULT_PRE_CANCEL_WINDOW: u64 = 60;

pub const TRADER_A_SEED: u64 = 1;
pub const TRADER_B_SEED: u64 = 2;
pub const TRADER_C_SEED: u64 = 3;
pub const TRADER_D_SEED: u64 = 4;

pub const TRADER_A_SELL_CLIENT_ID: u64 = 1;
pub const TRADER_B_SELL_CLIENT_ID: u64 = 2;
pub const TRADER_C_BUY_CLIENT_ID: u64 = 3;
pub const TRADER_A_BUY_CLIENT_ID: u64 = 4;

pub const TRADER_A_INITIAL_PRICE: u64 = 1_000;
pub const TRADER_A_INITIAL_SIZE: u64 = 10;
pub const TRADER_B_INITIAL_PRICE: u64 = 1_500;
pub const TRADER_B_INITIAL_SIZE: u64 = 20;
pub const TRADER_B_SIZE_DELTA: u64 = 10;
pub const TRADER_C_BUY_PRICE: u64 = 1_500;
pub const TRADER_C_BUY_SIZE: u64 = 8;
pub const TRADER_B_NEW_PRICE: u64 = 1_800;
pub const TRADER_B_NEW_SIZE: u64 = 2;
pub const TRADER_A_FINAL_PRICE: u64 = 1_800;
pub const TRADER_A_FINAL_SIZE: u64 = 10;

pub const TRADER_FUND_BASE: u64 = 1_000_000_000;
pub const TRADER_FUND_QUOTE: u64 = 1_000_000_000;

pub const EXPECTED_SCENARIO_TXNS: usize = 15;

const DEFAULT_PACKAGE_RELATIVE: &str =
    "Desktop/orderbook_poc/move/simple_market/build/simple_market";

pub struct ScenarioTxn {
    pub label: String,
    pub txn: SignedTransaction,
}

pub fn resolve_package_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("HYDRANGEA_MARKET_PACKAGE_DIR") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
        bail!(
            "package directory '{}' from HYDRANGEA_MARKET_PACKAGE_DIR does not exist",
            candidate.display()
        );
    }

    let workspace_candidate = PathBuf::from("move/simple_market/build/simple_market");
    if workspace_candidate.exists() {
        return Ok(workspace_candidate);
    }

    if let Ok(root) = env::var("ORDERBOOK_POC_ROOT") {
        let candidate = PathBuf::from(root).join("move/simple_market/build/simple_market");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    if let Ok(home) = env::var("HOME") {
        let candidate = PathBuf::from(home).join(DEFAULT_PACKAGE_RELATIVE);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!("unable to locate compiled simple_market package; set HYDRANGEA_MARKET_PACKAGE_DIR")
}

pub fn build_three_trader_transactions(
    package_dir: &Path,
    chain_id: ChainId,
) -> Result<Vec<ScenarioTxn>> {
    let mut trader_a = LocalAccount::generate(TRADER_A_SEED)?;
    let market_signer = LocalAccount::generate(TRADER_B_SEED)?;
    let mut trader_b = LocalAccount::generate(TRADER_C_SEED)?;
    let mut trader_c = LocalAccount::generate(TRADER_D_SEED)?;

    let module_owner = trader_a.address;
    let trader_a_address = trader_a.address;
    let trader_b_address = trader_b.address;
    let trader_c_address = trader_c.address;

    println!("Module owner address: {}", module_owner);
    println!(
        "Trader A: {} | Market signer: {} | Trader B: {} | Trader C: {}",
        trader_a_address, market_signer.address, trader_b_address, trader_c_address
    );
    let (metadata, modules) = load_package_artifacts(package_dir)?;

    let mut transactions = Vec::new();

    transactions.push(ScenarioTxn {
        label: "Publish simple_market package".to_string(),
        txn: publish_package(&mut trader_a, metadata, modules, chain_id)
            .context("publish package")?,
    });

    transactions.push(ScenarioTxn {
        label: "Create market (no self-match, emit events)".to_string(),
        txn: create_market(
            &mut trader_a,
            &market_signer,
            DEFAULT_ALLOW_SELF_MATCHING,
            DEFAULT_ALLOW_EVENTS_EMISSION,
            DEFAULT_PRE_CANCEL_WINDOW,
            chain_id,
        )
        .context("create market")?,
    });

    transactions.push(ScenarioTxn {
        label: "Register Trader A".to_string(),
        txn: register_trader(module_owner, &mut trader_a, chain_id).context("register trader A")?,
    });

    transactions.push(ScenarioTxn {
        label: "Register Trader B".to_string(),
        txn: register_trader(module_owner, &mut trader_b, chain_id).context("register trader B")?,
    });

    transactions.push(ScenarioTxn {
        label: "Register Trader C".to_string(),
        txn: register_trader(module_owner, &mut trader_c, chain_id).context("register trader C")?,
    });

    transactions.push(ScenarioTxn {
        label: "Mint Trader A demo balances".to_string(),
        txn: mint_trader_funds(
            &mut trader_a,
            trader_a_address,
            TRADER_FUND_BASE,
            TRADER_FUND_QUOTE,
            chain_id,
        )
        .context("mint trader A funds")?,
    });

    transactions.push(ScenarioTxn {
        label: "Mint Trader B demo balances".to_string(),
        txn: mint_trader_funds(
            &mut trader_a,
            trader_b_address,
            TRADER_FUND_BASE,
            TRADER_FUND_QUOTE,
            chain_id,
        )
        .context("mint trader B funds")?,
    });

    transactions.push(ScenarioTxn {
        label: "Mint Trader C demo balances".to_string(),
        txn: mint_trader_funds(
            &mut trader_a,
            trader_c_address,
            TRADER_FUND_BASE,
            TRADER_FUND_QUOTE,
            chain_id,
        )
        .context("mint trader C funds")?,
    });

    transactions.push(ScenarioTxn {
        label: format!(
            "Trader A places ask @ {} (size {})",
            TRADER_A_INITIAL_PRICE, TRADER_A_INITIAL_SIZE
        ),
        txn: place_limit_order_with_client_id(
            module_owner,
            &mut trader_a,
            &market_signer,
            TRADER_A_INITIAL_PRICE,
            TRADER_A_INITIAL_SIZE,
            false,
            TRADER_A_SELL_CLIENT_ID,
            chain_id,
        )
        .context("trader A initial ask")?,
    });

    transactions.push(ScenarioTxn {
        label: format!(
            "Trader B places ask @ {} (size {})",
            TRADER_B_INITIAL_PRICE, TRADER_B_INITIAL_SIZE
        ),
        txn: place_limit_order_with_client_id(
            module_owner,
            &mut trader_b,
            &market_signer,
            TRADER_B_INITIAL_PRICE,
            TRADER_B_INITIAL_SIZE,
            false,
            TRADER_B_SELL_CLIENT_ID,
            chain_id,
        )
        .context("trader B initial ask")?,
    });

    transactions.push(ScenarioTxn {
        label: "Trader A cancels ask".to_string(),
        txn: cancel_order_by_client_id(
            module_owner,
            &mut trader_a,
            &market_signer,
            TRADER_A_SELL_CLIENT_ID,
            chain_id,
        )
        .context("trader A cancel")?,
    });

    transactions.push(ScenarioTxn {
        label: format!("Trader B decreases ask by {}", TRADER_B_SIZE_DELTA),
        txn: decrease_order_size_by_client_id(
            module_owner,
            &mut trader_b,
            &market_signer,
            TRADER_B_SELL_CLIENT_ID,
            TRADER_B_SIZE_DELTA,
            chain_id,
        )
        .context("trader B decrease")?,
    });

    transactions.push(ScenarioTxn {
        label: format!(
            "Trader C places bid @ {} (size {})",
            TRADER_C_BUY_PRICE, TRADER_C_BUY_SIZE
        ),
        txn: place_limit_order_with_client_id(
            module_owner,
            &mut trader_c,
            &market_signer,
            TRADER_C_BUY_PRICE,
            TRADER_C_BUY_SIZE,
            true,
            TRADER_C_BUY_CLIENT_ID,
            chain_id,
        )
        .context("trader C buy")?,
    });

    transactions.push(ScenarioTxn {
        label: format!(
            "Trader B reprices ask @ {} (size {})",
            TRADER_B_NEW_PRICE, TRADER_B_NEW_SIZE
        ),
        txn: replace_order_by_client_id(
            module_owner,
            &mut trader_b,
            &market_signer,
            TRADER_B_SELL_CLIENT_ID,
            TRADER_B_NEW_PRICE,
            TRADER_B_NEW_SIZE,
            false,
            chain_id,
        )
        .context("trader B reprice")?,
    });

    transactions.push(ScenarioTxn {
        label: format!(
            "Trader A places bid @ {} (size {})",
            TRADER_A_FINAL_PRICE, TRADER_A_FINAL_SIZE
        ),
        txn: place_limit_order_with_client_id(
            module_owner,
            &mut trader_a,
            &market_signer,
            TRADER_A_FINAL_PRICE,
            TRADER_A_FINAL_SIZE,
            true,
            TRADER_A_BUY_CLIENT_ID,
            chain_id,
        )
        .context("trader A final buy")?,
    });

    Ok(transactions)
}

pub fn wait_for_execution_logs(path: &Path, expected: usize, timeout: Duration) -> Result<()> {
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

fn load_package_artifacts(package_dir: &Path) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    let metadata_path = package_dir.join("package-metadata.bcs");
    let metadata = std::fs::read(&metadata_path).with_context(|| {
        format!(
            "failed to read package metadata at {}",
            metadata_path.display()
        )
    })?;

    let modules_dir = package_dir.join("bytecode_modules");
    let mut module_paths = Vec::new();
    for entry in std::fs::read_dir(&modules_dir).with_context(|| {
        format!(
            "failed to list module directory at {}",
            modules_dir.display()
        )
    })? {
        let entry = entry
            .with_context(|| format!("failed to read entry inside {}", modules_dir.display()))?;
        module_paths.push(entry.path());
    }
    module_paths.sort();

    let mut modules = Vec::new();
    for path in module_paths {
        if path.extension().and_then(|ext| ext.to_str()) != Some("mv") {
            continue;
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("failed to read compiled module at {}", path.display()))?;
        modules.push(bytes);
    }

    if modules.is_empty() {
        bail!("no compiled modules found in {}", modules_dir.display());
    }

    Ok((metadata, modules))
}
