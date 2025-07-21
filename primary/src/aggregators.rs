// Copyright(C) Facebook, Inc. and its affiliates.
use crate::error::{DagError, DagResult};
use crate::messages::{Certificate, Header, Vote};
use blsttc::{PublicKeyShareG2, SignatureShareG1};
use config::{Committee, Stake};
use crypto::{aggregate_sign, Hash, PublicKey};
use std::collections::HashSet;

/// Aggregates votes for a particular header into a certificate.
pub struct VotesAggregator {
    weight: Stake,
    votes: Vec<(PublicKeyShareG2, SignatureShareG1)>,
    used: HashSet<PublicKey>,
    agg_sign: SignatureShareG1,
    pk_bit_vec: u128,
    is_qc_sent: bool,
}

impl VotesAggregator {
    pub fn new() -> Self {
        Self {
            weight: 0,
            votes: Vec::new(),
            used: HashSet::new(),
            agg_sign: SignatureShareG1::default(),
            pk_bit_vec: 0,
            is_qc_sent: false,
        }
    }

    pub fn append(
        &mut self,
        vote: Vote,
        committee: &Committee,
        header: &Header,
    ) -> DagResult<Option<Certificate>> {
        let author = vote.author;
        let author_bls_g2 = committee.get_bls_public_g2(&vote.author);

        // Ensure it is the first time this authority votes.
        ensure!(self.used.insert(author), DagError::AuthorityReuse(author));

        self.votes.push((author_bls_g2, vote.signature.clone()));
        self.weight += committee.stake(&author);

        if !self.is_qc_sent {
            // info!("verified vote for {}", vote.id);
            vote.verify(committee)?;

            if self.votes.len() == 1 {
                self.agg_sign = vote.signature;

                //adding it to bitvec
                self.pk_bit_vec |=
                    1 << committee.sorted_keys.binary_search(&author_bls_g2).unwrap();
            } else if self.votes.len() >= 2 {
                let new_agg_sign = aggregate_sign(&self.agg_sign, &vote.signature);
                self.agg_sign = new_agg_sign;

                //adding node id to bitvec
                self.pk_bit_vec |=
                    1 << committee.sorted_keys.binary_search(&author_bls_g2).unwrap();
            }

            if self.weight >= committee.validity_threshold() {
                self.weight = 0; // Ensures quorum is only reached once.

                // info!("{:b}", self.pk_bit_vec);
                self.is_qc_sent = true;

                return Ok(Some(Certificate {
                    id: header.digest(),
                    round: header.round,
                    origin: header.author,
                    votes: (self.pk_bit_vec, self.agg_sign.clone()),
                }));
            }
        }

        Ok(None)
    }
}
