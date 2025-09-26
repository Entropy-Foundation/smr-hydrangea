//! Helpers for constructing Aptos transactions used by tests and clients.

use crate::accounts::LocalAccount;
use anyhow::Result;
use aptos_cached_packages::aptos_stdlib;
use aptos_crypto::SigningKey;
use aptos_types::{
    chain_id::ChainId,
    transaction::{
        authenticator::AccountAuthenticator, EntryFunction, RawTransaction, RawTransactionWithData,
        SignedTransaction, TransactionPayload,
    },
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

/// Builds a multi-agent transaction that invokes `simple_market::market_setup::create_market`.
pub fn create_market(
    admin: &mut LocalAccount,
    market_signer: &LocalAccount,
    allow_self_matching: bool,
    allow_events_emission: bool,
    pre_cancellation_window_secs: u64,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(admin.address, Identifier::new("market_setup")?);
    let function = Identifier::new("create_market")?;
    let entry_function = EntryFunction::new(
        module,
        function,
        vec![],
        vec![
            bcs::to_bytes(&allow_self_matching)?,
            bcs::to_bytes(&allow_events_emission)?,
            bcs::to_bytes(&pre_cancellation_window_secs)?,
        ],
    );

    build_multi_agent_market_txn(admin, market_signer, entry_function, chain_id)
}

/// Builds a signed transaction that registers the demo trader for both market coins.
pub fn register_trader(
    module_owner: AccountAddress,
    trader: &mut LocalAccount,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(module_owner, Identifier::new("market_setup")?);
    let function = Identifier::new("register_trader")?;
    let entry_function = EntryFunction::new(module, function, vec![], vec![]);

    let payload = TransactionPayload::EntryFunction(entry_function);
    let raw_txn = RawTransaction::new(
        trader.address,
        trader.sequence_number,
        payload,
        2_000_000,
        100,
        default_expiration_secs(),
        chain_id,
    );

    trader.sign(raw_txn)
}

/// Builds a signed transaction that mints demo balances for the trader.
pub fn mint_trader_funds(
    admin: &mut LocalAccount,
    trader: AccountAddress,
    base_amount: u64,
    quote_amount: u64,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(admin.address, Identifier::new("market_setup")?);
    let function = Identifier::new("mint_to_trader")?;
    let entry_function = EntryFunction::new(
        module,
        function,
        vec![],
        vec![
            bcs::to_bytes(&trader)?,
            bcs::to_bytes(&base_amount)?,
            bcs::to_bytes(&quote_amount)?,
        ],
    );

    let payload = TransactionPayload::EntryFunction(entry_function);
    let raw_txn = RawTransaction::new(
        admin.address,
        admin.sequence_number,
        payload,
        2_000_000,
        100,
        default_expiration_secs(),
        chain_id,
    );

    admin.sign(raw_txn)
}

/// Builds a multi-agent transaction that invokes `place_limit_order_with_client_id`.
pub fn place_limit_order_with_client_id(
    module_owner: AccountAddress,
    trader: &mut LocalAccount,
    market_signer: &LocalAccount,
    limit_price: u64,
    size: u64,
    is_bid: bool,
    client_order_id: u64,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(module_owner, Identifier::new("market_setup")?);
    let function = Identifier::new("place_limit_order_with_client_id")?;
    let entry_function = EntryFunction::new(
        module,
        function,
        vec![],
        vec![
            bcs::to_bytes(&limit_price)?,
            bcs::to_bytes(&size)?,
            bcs::to_bytes(&is_bid)?,
            bcs::to_bytes(&client_order_id)?,
        ],
    );

    build_multi_agent_market_txn(trader, market_signer, entry_function, chain_id)
}

/// Builds a multi-agent transaction that cancels an order by client order ID.
pub fn cancel_order_by_client_id(
    module_owner: AccountAddress,
    trader: &mut LocalAccount,
    market_signer: &LocalAccount,
    client_order_id: u64,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(module_owner, Identifier::new("market_setup")?);
    let function = Identifier::new("cancel_order_by_client_id")?;
    let entry_function = EntryFunction::new(
        module,
        function,
        vec![],
        vec![bcs::to_bytes(&client_order_id)?],
    );

    build_multi_agent_market_txn(trader, market_signer, entry_function, chain_id)
}

/// Builds a multi-agent transaction that decreases an order size by client order ID.
pub fn decrease_order_size_by_client_id(
    module_owner: AccountAddress,
    trader: &mut LocalAccount,
    market_signer: &LocalAccount,
    client_order_id: u64,
    size_delta: u64,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(module_owner, Identifier::new("market_setup")?);
    let function = Identifier::new("decrease_order_size_by_client_id")?;
    let entry_function = EntryFunction::new(
        module,
        function,
        vec![],
        vec![
            bcs::to_bytes(&client_order_id)?,
            bcs::to_bytes(&size_delta)?,
        ],
    );

    build_multi_agent_market_txn(trader, market_signer, entry_function, chain_id)
}

/// Builds a multi-agent transaction that replaces an order by client order ID.
pub fn replace_order_by_client_id(
    module_owner: AccountAddress,
    trader: &mut LocalAccount,
    market_signer: &LocalAccount,
    client_order_id: u64,
    limit_price: u64,
    size: u64,
    is_bid: bool,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let module = ModuleId::new(module_owner, Identifier::new("market_setup")?);
    let function = Identifier::new("replace_order_by_client_id")?;
    let entry_function = EntryFunction::new(
        module,
        function,
        vec![],
        vec![
            bcs::to_bytes(&client_order_id)?,
            bcs::to_bytes(&limit_price)?,
            bcs::to_bytes(&size)?,
            bcs::to_bytes(&is_bid)?,
        ],
    );

    build_multi_agent_market_txn(trader, market_signer, entry_function, chain_id)
}

fn build_multi_agent_market_txn(
    primary: &mut LocalAccount,
    market_signer: &LocalAccount,
    entry_function: EntryFunction,
    chain_id: ChainId,
) -> Result<SignedTransaction> {
    let payload = TransactionPayload::EntryFunction(entry_function);
    let raw_txn = RawTransaction::new(
        primary.address,
        primary.sequence_number,
        payload,
        2_000_000,
        100,
        default_expiration_secs(),
        chain_id,
    );

    let secondary_addresses = vec![market_signer.address];
    let message =
        RawTransactionWithData::new_multi_agent(raw_txn.clone(), secondary_addresses.clone());

    let primary_signature = primary.private_key.sign(&message)?;
    let primary_authenticator =
        AccountAuthenticator::ed25519(primary.public_key.clone(), primary_signature);

    let market_signature = market_signer.private_key.sign(&message)?;
    let market_authenticator =
        AccountAuthenticator::ed25519(market_signer.public_key.clone(), market_signature);

    primary.sequence_number += 1;

    Ok(SignedTransaction::new_multi_agent(
        raw_txn,
        primary_authenticator,
        secondary_addresses,
        vec![market_authenticator],
    ))
}
