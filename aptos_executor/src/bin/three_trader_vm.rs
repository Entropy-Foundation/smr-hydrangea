use anyhow::{bail, Context, Result};
use aptos_executor::{
    scenarios::three_trader::{
        build_three_trader_transactions, resolve_package_dir, EXPECTED_SCENARIO_TXNS,
        TRADER_A_SEED, TRADER_B_SEED, TRADER_C_SEED, TRADER_D_SEED,
    },
    AptosVmExecutor, LocalAccount,
};
use aptos_types::vm_status::VMStatus;

const INITIAL_BOOTSTRAP_BALANCE: u64 = 1_000_000_000_000;

fn main() -> Result<()> {
    let package_dir = resolve_package_dir()?;
    println!(
        "Loading simple_market package from {}",
        package_dir.display()
    );

    let mut executor = AptosVmExecutor::new().context("failed to construct Aptos VM executor")?;
    bootstrap_deterministic_accounts(&executor)?;

    let chain_id = executor.chain_id();
    let scenario = build_three_trader_transactions(&package_dir, chain_id)?;
    if scenario.len() != EXPECTED_SCENARIO_TXNS {
        bail!(
            "three trader scenario produced {} transactions, expected {}",
            scenario.len(),
            EXPECTED_SCENARIO_TXNS
        );
    }

    println!("Executing three-trader demo via Aptos VM...");
    for (index, scenario_txn) in scenario.into_iter().enumerate() {
        let label = scenario_txn.label;
        let txns = vec![scenario_txn.txn];
        let mut results = executor.execute_block(&txns);
        let result = results
            .pop()
            .context("VM executor returned no result for transaction")?;

        match result.status() {
            VMStatus::Executed => {
                println!(
                    "  ✓ Step {}: {} (gas used: {})",
                    index + 1,
                    label,
                    result.gas_used()
                );
            }
            status => {
                bail!(
                    "step {} ({}) failed with status {:?}",
                    index + 1,
                    label,
                    status
                );
            }
        }
    }

    println!("All scenario transactions executed successfully via Aptos VM.");
    Ok(())
}

fn bootstrap_deterministic_accounts(executor: &AptosVmExecutor) -> Result<()> {
    let seeds = [TRADER_A_SEED, TRADER_B_SEED, TRADER_C_SEED, TRADER_D_SEED];
    for seed in seeds {
        let account = LocalAccount::generate(seed)
            .with_context(|| format!("failed to generate account for seed {}", seed))?;
        executor.bootstrap_account(&account, INITIAL_BOOTSTRAP_BALANCE);
    }
    Ok(())
}
