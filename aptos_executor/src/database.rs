//! State management utilities for the Aptos VM integration.

use crate::accounts::LocalAccount;
use anyhow::{anyhow, Result};
use aptos_crypto::HashValue;
use aptos_storage_interface::{
    state_store::state_view::db_state_view::{DbStateView, LatestDbStateCheckpointView},
    DbReader, Result as StorageResult,
};
use aptos_types::{
    account_config::{
        primary_apt_store, AccountResource, AggregatorResource, CoinStoreResource,
        ConcurrentSupplyResource, FungibleStoreResource, ObjectGroupResource,
    },
    event::{EventHandle, EventKey},
    state_store::{state_key::StateKey, state_value::StateValue},
    transaction::Version,
    utility_coin::AptosCoinType,
    write_set::{TransactionWrite, WriteOp},
};
use aptos_vm_genesis::{generate_genesis_change_set_for_mainnet, GenesisOptions};
use move_core_types::{account_address::AccountAddress, move_resource::MoveStructType};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};

/// Lightweight in-memory implementation of the Aptos `DbReader` trait tailored for tests.
#[derive(Default)]
pub struct TestDbReader {
    states: RwLock<HashMap<StateKey, StateValue>>,
    version: AtomicU64,
}

impl TestDbReader {
    pub fn new() -> Self {
        Self {
            states: RwLock::new(HashMap::new()),
            version: AtomicU64::new(0),
        }
    }

    /// Inserts or replaces the value associated with the given state key.
    pub fn set_state_value(&self, key: StateKey, value: StateValue) {
        self.states.write().unwrap().insert(key, value);
    }

    /// Removes the value associated with the given state key, if any.
    pub fn remove_state_value(&self, key: &StateKey) {
        self.states.write().unwrap().remove(key);
    }

    /// Reads the current value for a state key, if one exists.
    pub fn get_state_value(&self, key: &StateKey) -> Option<StateValue> {
        self.states.read().unwrap().get(key).cloned()
    }

    /// Returns the latest state version recorded by the reader.
    pub fn latest_version(&self) -> Version {
        self.version.load(Ordering::SeqCst)
    }

    /// Applies a single write operation directly into the in-memory store.
    fn apply_write_op(&self, key: StateKey, write: &WriteOp) {
        if write.is_delete() {
            self.remove_state_value(&key);
            return;
        }

        match write.as_state_value() {
            Some(state_value) => {
                self.set_state_value(key, state_value);
            }
            None => {
                eprintln!("Ignoring write op without state value for key {:?}", key);
            }
        }
    }

    fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
    }
}

impl DbReader for TestDbReader {
    fn get_latest_state_checkpoint_version(&self) -> StorageResult<Option<Version>> {
        Ok(Some(self.version.load(Ordering::SeqCst)))
    }

    fn get_state_proof_by_version_ext(
        &self,
        _key_hash: &HashValue,
        _version: Version,
        _root_depth: usize,
    ) -> StorageResult<aptos_types::proof::SparseMerkleProofExt> {
        Ok(aptos_types::proof::SparseMerkleProofExt::new(None, vec![]))
    }

    fn get_state_value_by_version(
        &self,
        state_key: &StateKey,
        _version: Version,
    ) -> StorageResult<Option<StateValue>> {
        Ok(self.states.read().unwrap().get(state_key).cloned())
    }

    fn get_state_value_with_version_by_version(
        &self,
        state_key: &StateKey,
        version: Version,
    ) -> StorageResult<Option<(Version, StateValue)>> {
        Ok(self
            .get_state_value_by_version(state_key, version)?
            .map(|value| (version, value)))
    }
}

impl LatestDbStateCheckpointView for TestDbReader {
    fn latest_state_checkpoint_view(
        &self,
    ) -> aptos_types::state_store::StateViewResult<DbStateView> {
        let version = self.version.load(Ordering::SeqCst);
        let snapshot = Arc::new(TestDbReader {
            states: RwLock::new(self.states.read().unwrap().clone()),
            version: AtomicU64::new(version),
        });

        use aptos_storage_interface::state_store::state_view::db_state_view::DbStateViewAtVersion;
        let dyn_reader: Arc<dyn DbReader> = snapshot;
        dyn_reader.state_view_at_version(Some(version))
    }
}

/// Convenience wrapper that provides higher-level helpers on top of `TestDbReader`.
pub struct AptosDatabase {
    reader: Arc<TestDbReader>,
}

impl AptosDatabase {
    /// Builds a fresh database populated with the Aptos mainnet genesis change set.
    pub fn new_with_genesis() -> Result<Self> {
        let reader = Arc::new(TestDbReader::new());
        Self::apply_genesis(&reader)?;
        Ok(Self { reader })
    }

    /// Returns a shared reference to the underlying reader.
    pub fn reader(&self) -> Arc<TestDbReader> {
        Arc::clone(&self.reader)
    }

    /// Creates a `DbStateView` snapshot suitable for VM execution.
    pub fn state_view(&self) -> DbStateView {
        self.reader
            .latest_state_checkpoint_view()
            .expect("latest_state_checkpoint_view should succeed")
    }

    /// Fetches a raw state value for the provided key, if present.
    pub fn get_state_value(&self, key: &StateKey) -> Option<StateValue> {
        self.reader.get_state_value(key)
    }

    /// Applies the writes produced by a VM output back into the in-memory store.
    pub fn apply_vm_output(&self, output: &aptos_vm_types::output::VMOutput) {
        let tx_output = output
            .clone()
            .into_transaction_output()
            .expect("VM output should convert into transaction output");

        for (state_key, write_op) in tx_output.write_set().write_op_iter() {
            self.reader.apply_write_op(state_key.clone(), write_op);
        }

        self.reader.bump_version();
    }

    /// Publishes account resources and an APT balance for the provided local account.
    pub fn publish_account_resources(&self, account: &LocalAccount, initial_balance: u64) {
        use aptos_types::transaction::authenticator::AuthenticationKey;

        let auth_key = AuthenticationKey::ed25519(&account.public_key);
        let account_resource = AccountResource::new(
            account.sequence_number,
            auth_key.to_vec(),
            EventHandle::new(EventKey::new(0, account.address), 0),
            EventHandle::new(EventKey::new(1, account.address), 0),
        );

        let account_key = StateKey::resource(&account.address, &AccountResource::struct_tag())
            .expect("AccountResource should serialize");
        let account_bytes = bcs::to_bytes(&account_resource).expect("AccountResource BCS");
        self.reader
            .set_state_value(account_key, StateValue::new_legacy(account_bytes.into()));

        // include an extra buffer for gas so the first transaction never fails
        let mut effective_balance = initial_balance;
        if effective_balance > 0 {
            effective_balance = effective_balance.saturating_add(1_000_000_000);
        }

        self.publish_coin_store(account.address, effective_balance);
        self.publish_fungible_store(account.address, effective_balance);
        self.reader.bump_version();
    }

    fn apply_genesis(reader: &Arc<TestDbReader>) -> Result<()> {
        let genesis_change_set = generate_genesis_change_set_for_mainnet(GenesisOptions::Head);
        for (state_key, write_op) in genesis_change_set.write_set().write_op_iter() {
            reader.apply_write_op(state_key.clone(), write_op);
        }
        reader.bump_version();
        Self::ensure_apt_supply(reader)?;
        Ok(())
    }

    fn publish_coin_store(
        &self,
        account_address: move_core_types::account_address::AccountAddress,
        balance: u64,
    ) {
        let deposit_events = EventHandle::new(EventKey::new(2, account_address), 0);
        let withdraw_events = EventHandle::new(EventKey::new(3, account_address), 0);
        let coin_store = CoinStoreResource::<AptosCoinType>::new(
            balance,
            false,
            deposit_events,
            withdraw_events,
        );

        let coin_store_key = StateKey::resource(
            &account_address,
            &CoinStoreResource::<AptosCoinType>::struct_tag(),
        )
        .expect("CoinStore resource key");
        let coin_store_bytes = bcs::to_bytes(&coin_store).expect("CoinStore BCS");
        self.reader.set_state_value(
            coin_store_key,
            StateValue::new_legacy(coin_store_bytes.into()),
        );
    }

    fn publish_fungible_store(
        &self,
        account_address: move_core_types::account_address::AccountAddress,
        balance: u64,
    ) {
        let primary_store_address = primary_apt_store(account_address);
        let mut object_group = ObjectGroupResource::default();
        let store = FungibleStoreResource::new(AccountAddress::TEN, balance, false);
        object_group.insert(
            FungibleStoreResource::struct_tag(),
            bcs::to_bytes(&store).expect("fungible store BCS"),
        );
        let group_bytes = object_group
            .to_bytes()
            .expect("fungible store object group serialization");
        let group_key =
            StateKey::resource_group(&primary_store_address, &ObjectGroupResource::struct_tag());
        self.reader
            .set_state_value(group_key, StateValue::new_legacy(group_bytes.into()));
    }

    fn ensure_apt_supply(reader: &Arc<TestDbReader>) -> Result<()> {
        use move_core_types::{
            account_address::AccountAddress as MoveAddress, identifier::Identifier,
            language_storage::StructTag,
        };

        #[derive(serde::Serialize, serde::Deserialize)]
        struct Supply {
            current: u128,
            maximum: Option<u128>,
        }

        let supply_tag = StructTag {
            address: MoveAddress::ONE,
            module: Identifier::new("fungible_asset")?,
            name: Identifier::new("Supply")?,
            type_args: vec![],
        };

        let object_group_key =
            StateKey::resource_group(&MoveAddress::TEN, &ObjectGroupResource::struct_tag());

        let mut group: BTreeMap<StructTag, Vec<u8>> = reader
            .get_state_value(&object_group_key)
            .map(|value| bcs::from_bytes(value.bytes()))
            .transpose()
            .map_err(|e| anyhow!("failed to decode APT supply object group: {e}"))?
            .unwrap_or_default();

        let initial_supply = 1_000_000_000_000_000_000u128;
        group.insert(
            supply_tag.clone(),
            bcs::to_bytes(&Supply {
                current: initial_supply,
                maximum: None,
            })?,
        );

        let concurrent_supply = ConcurrentSupplyResource {
            current: AggregatorResource::new(initial_supply, u128::MAX),
        };
        group.insert(
            ConcurrentSupplyResource::struct_tag(),
            bcs::to_bytes(&concurrent_supply)?,
        );

        let serialized = bcs::to_bytes(&group)?;
        reader.set_state_value(object_group_key, StateValue::new_legacy(serialized.into()));
        reader.bump_version();
        Ok(())
    }
}
