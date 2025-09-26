//! Helpers for constructing Aptos transactions used by tests and clients.

use crate::accounts::LocalAccount;
use anyhow::Result;
use aptos_cached_packages::aptos_stdlib;
use aptos_types::{
    chain_id::ChainId,
    transaction::{EntryFunction, RawTransaction, SignedTransaction, TransactionPayload},
};
use move_core_types::{
    account_address::AccountAddress,
    identifier::Identifier,
    language_storage::{ModuleId, StructTag, TypeTag},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Builds a signed transaction that transfers APT from `sender` to `recipient`.
pub fn apt_transfer(
    sender: &mut LocalAccount,
    recipient: AccountAddress,
    amount: u64,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(AccountAddress::ONE, Identifier::new("coin")?);
    let function = Identifier::new("transfer")?;
    let coin_type = TypeTag::Struct(Box::new(StructTag {
        address: AccountAddress::ONE,
        module: Identifier::new("aptos_coin")?,
        name: Identifier::new("AptosCoin")?,
        type_args: vec![],
    }));

    let entry_function = EntryFunction::new(
        module,
        function,
        vec![coin_type],
        vec![bcs::to_bytes(&recipient)?, bcs::to_bytes(&amount)?],
    );

    let payload = TransactionPayload::EntryFunction(entry_function);
    let expiration_secs = default_expiration_secs();

    let raw_txn = RawTransaction::new(
        sender.address,
        sender.sequence_number,
        payload,
        2_000_000,
        100,
        expiration_secs,
        chain_id,
    );

    sender.sign(raw_txn)
}

fn default_expiration_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .saturating_add(Duration::from_secs(600))
        .as_secs()
}

/// Builds a signed transaction that publishes a Move package via `code::publish_package_txn`.
pub fn publish_package(
    sender: &mut LocalAccount,
    metadata: Vec<u8>,
    modules: Vec<Vec<u8>>,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let payload = aptos_stdlib::code_publish_package_txn(metadata, modules);
    let raw_txn = RawTransaction::new(
        sender.address,
        sender.sequence_number,
        payload,
        2_000_000,
        100,
        default_expiration_secs(),
        chain_id,
    );

    sender.sign(raw_txn)
}
