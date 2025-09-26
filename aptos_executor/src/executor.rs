//! Aptos VM executor for running committed transactions.

use crate::{accounts::LocalAccount, database::AptosDatabase};
use anyhow::{anyhow, bail, Result};
use aptos_types::{
    account_config::{
        primary_apt_store, CoinStoreResource, ConcurrentFungibleBalanceResource,
        FungibleStoreResource, ObjectGroupResource,
    },
    chain_id::ChainId,
    state_store::{state_key::StateKey, TStateView},
    transaction::{AuxiliaryInfo, AuxiliaryInfoTrait, SignedTransaction},
    utility_coin::AptosCoinType,
    vm_status::VMStatus,
};
use aptos_vm::{data_cache::AsMoveResolver, AptosVM};
use aptos_vm_environment::environment::AptosEnvironment;
use aptos_vm_logging::log_schema::AdapterLogSchema;
use aptos_vm_types::module_and_script_storage::AsAptosCodeStorage;
use move_core_types::{account_address::AccountAddress, move_resource::MoveStructType};

/// Result of executing a single transaction through the VM.
pub struct TransactionResult {
    pub status: VMStatus,
    pub output: aptos_vm_types::output::VMOutput,
}

impl TransactionResult {
    pub fn gas_used(&self) -> u64 {
        self.output.gas_used()
    }

    pub fn status(&self) -> &VMStatus {
        &self.status
    }
}

/// High-level executor that wires state management, VM construction, and
/// account setup together for the node integration.
pub struct AptosVmExecutor {
    database: AptosDatabase,
    chain_id: ChainId,
}

impl AptosVmExecutor {
    /// Constructs a new executor with Aptos genesis state.
    pub fn new() -> Result<Self> {
        let database = AptosDatabase::new_with_genesis()?;
        Ok(Self {
            database,
            chain_id: ChainId::test(),
        })
    }

    /// Returns the configured chain id.
    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Provides access to the underlying database for custom setup tasks.
    pub fn database(&self) -> &AptosDatabase {
        &self.database
    }

    /// Publishes account resources and funds the account with the provided balance.
    pub fn bootstrap_account(&self, account: &LocalAccount, initial_balance: u64) {
        self.database
            .publish_account_resources(account, initial_balance);
    }

    /// Executes a batch of transactions sequentially, applying each output to the in-memory state.
    pub fn execute_block(&mut self, txns: &[SignedTransaction]) -> Vec<TransactionResult> {
        let mut results = Vec::with_capacity(txns.len());
        for txn in txns {
            let state_view = self.database.state_view();
            let environment = AptosEnvironment::new(&state_view);
            let vm = AptosVM::new(&environment, &state_view);
            let storage_adapter = state_view.as_move_resolver();
            let module_storage = state_view.as_aptos_code_storage(&environment);
            let log_context = AdapterLogSchema::new(state_view.id(), 0);
            let auxiliary_info = AuxiliaryInfo::new_empty();

            let (status, output) = vm.execute_user_transaction(
                &storage_adapter,
                &module_storage,
                txn,
                &log_context,
                &auxiliary_info,
            );

            self.database.apply_vm_output(&output);
            results.push(TransactionResult { status, output });
        }
        results
    }

    /// Returns the fungible balance for the provided account, if present.
    pub fn account_balance(&self, address: AccountAddress) -> Result<u128> {
        let primary_store = primary_apt_store(address);
        let object_group_key =
            StateKey::resource_group(&primary_store, &ObjectGroupResource::struct_tag());
        if let Some(state_value) = self.database.get_state_value(&object_group_key) {
            let object_group: ObjectGroupResource = bcs::from_bytes(state_value.bytes())?;
            let mut fungible_balance = 0u128;

            if let Some(bytes) = object_group.group.get(&FungibleStoreResource::struct_tag()) {
                let store: FungibleStoreResource = bcs::from_bytes(bytes)?;
                fungible_balance += u128::from(store.balance());
            }

            if let Some(bytes) = object_group
                .group
                .get(&ConcurrentFungibleBalanceResource::struct_tag())
            {
                let concurrent: ConcurrentFungibleBalanceResource = bcs::from_bytes(bytes)?;
                fungible_balance += u128::from(concurrent.balance());
            }

            if fungible_balance > 0 {
                return Ok(fungible_balance);
            }
        }

        let coin_key =
            StateKey::resource(&address, &CoinStoreResource::<AptosCoinType>::struct_tag())
                .map_err(|_| anyhow!("failed to derive coin store key"))?;
        let Some(state_value) = self.database.get_state_value(&coin_key) else {
            bail!("account {:?} missing coin or fungible store", address);
        };

        let coin_store: CoinStoreResource<AptosCoinType> = bcs::from_bytes(state_value.bytes())?;
        Ok(u128::from(coin_store.coin()))
    }
}
