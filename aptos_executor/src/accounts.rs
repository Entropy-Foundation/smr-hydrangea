//! Account utilities for constructing Aptos transactions in tests and demos.

use anyhow::Result;
use aptos_crypto::ed25519::{Ed25519PrivateKey, Ed25519PublicKey};
use aptos_crypto::{hash::HashValue, PrivateKey};
use aptos_types::transaction::{RawTransaction, SignedTransaction};
use move_core_types::account_address::AccountAddress;
use std::convert::TryFrom;

/// Lightweight representation of an Aptos account with local signing keys.
pub struct LocalAccount {
    pub address: AccountAddress,
    pub private_key: Ed25519PrivateKey,
    pub public_key: Ed25519PublicKey,
    pub sequence_number: u64,
}

impl LocalAccount {
    /// Generates a deterministic account from a numeric seed.
    pub fn generate(seed: u64) -> Result<Self> {
        let seed_bytes = HashValue::sha3_256_of(&seed.to_le_bytes());
        let private_key = Ed25519PrivateKey::try_from(&seed_bytes.as_ref()[..])
            .map_err(|e| anyhow::anyhow!("failed to derive deterministic key: {e}"))?;
        Ok(Self::from_private_key(private_key, 0))
    }

    /// Creates an account wrapper from an existing private key.
    pub fn from_private_key(private_key: Ed25519PrivateKey, sequence_number: u64) -> Self {
        let public_key = private_key.public_key();
        let address =
            aptos_types::transaction::authenticator::AuthenticationKey::ed25519(&public_key)
                .account_address();
        Self {
            address,
            private_key,
            public_key,
            sequence_number,
        }
    }

    /// Signs the provided raw transaction, incrementing the local sequence number.
    pub fn sign(&mut self, raw_txn: RawTransaction) -> Result<SignedTransaction> {
        let signed = raw_txn.sign(&self.private_key, self.public_key.clone())?;
        self.sequence_number += 1;
        Ok(signed.into_inner())
    }
}
