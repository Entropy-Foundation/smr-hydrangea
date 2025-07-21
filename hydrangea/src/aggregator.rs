use crate::consensus::Round;
use crate::error::ConsensusResult;
use crate::messages::{Timeout, Vote, VoteType, QC, TC, WQC};
use blsttc::{PublicKeyShareG2, SignatureShareG1};
use config::{Committee, Stake};
use crypto::{
    aggregate_sign, combine_key_from_ids, remove_pubkeys, Digest, Hash, PublicKey, Signature,
};
use log::{debug, info};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

// #[cfg(test)]
// #[path = "tests/aggregator_tests.rs"]
// pub mod aggregator_tests;

pub struct Aggregator {
    committee: Committee,
    // Proposals indexed by round and block digest.
    votes_aggregators: HashMap<Round, HashMap<Digest, Box<QCMaker>>>,
    commit_aggregators: HashMap<Round, Box<QCMaker>>,
    timeouts_aggregators: HashMap<Round, Box<TCMaker>>,
}

impl Aggregator {
    pub fn new(committee: Committee) -> Self {
        Self {
            committee,
            votes_aggregators: HashMap::new(),
            commit_aggregators: HashMap::new(),
            timeouts_aggregators: HashMap::new(),
        }
    }

    pub fn add_normal_vote(&mut self, vote: Vote) -> ConsensusResult<Option<QC>> {
        // TODO [issue #7]: A bad node may make us run out of memory by sending many votes
        // with different round numbers or different digests.

        // Add the new vote to our aggregator and see if we have a QC.
        // We create one aggregator for each block, which handles the different types
        // of Votes internally.
        let total_nodes = self.committee.n as usize;
        self.votes_aggregators
            .entry(vote.round)
            .or_insert_with(HashMap::new)
            .entry(vote.blk_hash.clone())
            .or_insert_with(|| Box::new(QCMaker::new(total_nodes)))
            .append(vote, &self.committee, true)
    }

    pub fn add_commit_vote(&mut self, vote: Vote) -> ConsensusResult<Option<QC>> {
        let total_nodes = self.committee.n as usize;
        self.commit_aggregators
            .entry(vote.round)
            .or_insert_with(|| Box::new(QCMaker::new(total_nodes)))
            .append(vote, &self.committee, false)
    }

    pub fn add_timeout(&mut self, timeout: Timeout) -> ConsensusResult<(Stake, Option<TC>)> {
        // TODO: A bad node may make us run out of memory by sending many timeouts
        // with different round numbers.
        // Add the new timeout to our aggregator and see if we have a TC.
        self.timeouts_aggregators
            .entry(timeout.round)
            .or_insert_with(|| Box::new(TCMaker::new()))
            .append(timeout, &self.committee)
    }

    pub fn cleanup_prepares(&mut self, r: &Round) {
        self.votes_aggregators
            .retain(|block_round, _| block_round > r);
        self.commit_aggregators
            .retain(|block_round, _| block_round > r);
    }

    pub fn cleanup_timeouts(&mut self, round: &Round) {
        self.timeouts_aggregators.retain(|k, _| k > round);
    }
}

struct QCMaker {
    used: HashSet<PublicKey>,
    votes: Vec<(PublicKeyShareG2, SignatureShareG1)>,
    weight: Stake,
    agg_sign: SignatureShareG1,
    pk_bit_vec: Vec<u128>,
    is_qc_formed: bool,
}

impl QCMaker {
    pub fn new(total_nodes: usize) -> Self {
        Self {
            used: HashSet::new(),
            votes: Vec::new(),
            weight: 0,
            agg_sign: SignatureShareG1::default(),
            pk_bit_vec: vec![u128::MAX; (total_nodes + 127) / 128],
            is_qc_formed: false,
        }
    }

    fn is_valid(&self, vote: &Vote) -> bool {
        !self.used.contains(&vote.author)
    }

    /// Try to append a signature to a (partial) quorum.
    pub fn append(
        &mut self,
        vote: Vote,
        committee: &Committee,
        check_fast_threshold: bool,
    ) -> ConsensusResult<Option<QC>> {
        let author = vote.author;
        let author_bls_g2 = committee.get_bls_public_g2(&vote.author);
        if self.is_valid(&vote) {
            // Verify the signature and voting rights before storing to prevent DoS
            // by unauthorised nodes. Verification is done after membership check on
            // self.used to prevent authorised but Byzantine nodes from draining compute
            // by sending duplicate Votes (HashMap membership checks are cheap, sig
            // verification is more expensive).

            self.used.insert(author);
            self.votes.push((author_bls_g2, vote.signature.clone()));

            if !self.is_qc_formed {
                let id = committee.sorted_keys.binary_search(&author_bls_g2).unwrap();
                let chunk = id / 128;
                let bit = id % 128;
                self.pk_bit_vec[chunk] &= !(1 << bit);

                if self.votes.len() == 1 {
                    self.agg_sign = vote.signature;
                } else if self.votes.len() >= 2 {
                    let new_agg_sign = aggregate_sign(&self.agg_sign, &vote.signature);
                    self.agg_sign = new_agg_sign;
                }

                self.weight += committee.stake(&author);
                if self.weight == committee.quorum_threshold() {
                    // self.weight = 0; // Ensures QC of this type is only made once.
                    if !check_fast_threshold {
                        self.is_qc_formed = true;
                    }

                    let mut ids = Vec::new();

                    for idx in 0..committee.size() {
                        let x = idx / 128;
                        let chunk = self.pk_bit_vec[x];
                        let ridx = idx - x * 128;
                        if chunk & 1 << ridx != 0 {
                            ids.push(idx);
                        }
                    }
                    let agg_pk =
                        remove_pubkeys(&committee.combined_pubkey, ids, &committee.sorted_keys);
                    // SignatureShareG1::verify_batch(&vote.digest().0, &agg_pk, &self.agg_sign)?;

                    info!("Constructed {} QC. Votes: {} ", vote.kind, self.votes.len(),);

                    return Ok(Some(QC {
                        blk_hash: vote.blk_hash.clone(),
                        kind: vote.kind,
                        round: vote.round,
                        votes: (self.pk_bit_vec.clone(), self.agg_sign.clone()),
                        fast_quorum: false,
                    }));
                } else if self.weight == committee.fast_commit_threshold() {
                    self.weight = 0;
                    self.is_qc_formed = true;
                    let mut ids = Vec::new();

                    for idx in 0..committee.size() {
                        let x = idx / 128;
                        let chunk = self.pk_bit_vec[x];
                        let ridx = idx - x * 128;
                        if chunk & 1 << ridx != 0 {
                            ids.push(idx);
                        }
                    }

                    let agg_pk =
                        remove_pubkeys(&committee.combined_pubkey, ids, &committee.sorted_keys);
                    SignatureShareG1::verify_batch(&vote.digest().0, &agg_pk, &self.agg_sign)?;
                    info!(
                        "Constructed {} Fast path quorum. Votes: {} ",
                        vote.kind,
                        self.votes.len()
                    );

                    return Ok(Some(QC {
                        blk_hash: vote.blk_hash.clone(),
                        kind: vote.kind,
                        round: vote.round,
                        votes: (self.pk_bit_vec.clone(), self.agg_sign.clone()),
                        fast_quorum: true,
                    }));
                }
            }
        }

        Ok(None)
    }
}

struct TCMaker {
    high_qc: QC,
    high_wqc: WQC,
    used: HashSet<PublicKey>,
    votes: Vec<(PublicKey, Signature, Round)>,
    high_votes: Vec<Vote>,
    weight: Stake,
}

impl TCMaker {
    pub fn new() -> Self {
        Self {
            high_qc: QC::genesis(),
            high_wqc: WQC::genesis(),
            used: HashSet::new(),
            votes: Vec::new(),
            high_votes: Vec::new(),
            weight: 0,
        }
    }

    fn check_wqc(&mut self, committee: &Committee) -> Option<WQC> {
        let weak_cert_threshold = (committee.f + committee.p + 1) as usize;
        let mut freq_map = HashMap::new();
        for hv in &self.high_votes {
            freq_map
                .entry(hv.round)
                .or_insert_with(Vec::new)
                .push(hv.clone());
        }

        if let Some(votes) = freq_map
            .into_iter()
            .find(|(_, votes)| votes.len() >= weak_cert_threshold)
            .map(|(_, votes)| votes)
        {
            let first_vote = votes[0].clone();
            let mut agg_sign = first_vote.signature.clone();
            let blk_hash = first_vote.blk_hash;
            let round = first_vote.round;
            let mut pk_bit_vec: Vec<u128> = vec![u128::MAX; (committee.n as usize + 127) / 128];
            let mut id;
            let mut author_bls_g2;

            let mut first = true;
            let mut ids = Vec::new();
            for vote in &votes {
                author_bls_g2 = committee.get_bls_public_g2(&vote.author);
                id = committee.sorted_keys.binary_search(&author_bls_g2).unwrap();
                let chunk = id / 128;
                let bit = id % 128;
                pk_bit_vec[chunk] &= !(1 << bit);
                ids.push(id);
                if first {
                    first = false;
                } else {
                    let new_agg_sign = aggregate_sign(&agg_sign, &vote.signature);
                    agg_sign = new_agg_sign;
                }
            }

            let agg_pk = combine_key_from_ids(ids, &committee.sorted_keys);
            SignatureShareG1::verify_batch(&votes.last().unwrap().digest().0, &agg_pk, &agg_sign)
                .ok()?;

            return Some(WQC {
                blk_hash: blk_hash,
                round: round,
                votes: (pk_bit_vec, agg_sign),
            });
        }
        None
    }

    /// Try to append a signature to a (partial) quorum.
    pub fn append(
        &mut self,
        timeout: Timeout,
        committee: &Committee,
    ) -> ConsensusResult<(Stake, Option<TC>)> {
        let author = timeout.author;

        if !self.used.contains(&author) {
            // Verify the signature and voting rights before storing to prevent DoS
            // by unauthorised nodes. Verification is done after membership check on
            // self.used to prevent authorised but Byzantine nodes from draining compute
            // by sending duplicate Timeouts (HashMap membership checks are cheap, sig
            // verification is more expensive).
            timeout.is_well_formed(committee)?;
            // Ensure we ignore duplicates.
            self.used.insert(author);

            // Add the timeout to the accumulator.
            self.votes
                .push((author, timeout.signature, timeout.high_qc.round));
            self.high_votes.push(timeout.high_vote);
            self.weight += committee.stake(&author);

            // Update high QC.
            if timeout.high_qc.round > self.high_qc.round {
                self.high_qc = timeout.high_qc;
            }

            if timeout.high_wqc.round > self.high_wqc.round {
                self.high_wqc = timeout.high_wqc;
            }

            if self.weight >= committee.quorum_threshold() {
                // We do not reset the weight after creating the TC because we might
                // still need to send Timeout messages for this round to our honest
                // peers in case they either were censored by the Byzantine nodes or
                // the network dropped some of the honest Timeouts en-route to them.

                if let Some(wqc) = self.check_wqc(committee) {
                    if wqc.round > self.high_wqc.round {
                        self.high_wqc = wqc;
                    }
                }

                return Ok((
                    self.weight,
                    Some(TC {
                        high_qc: self.high_qc.clone(),
                        high_wqc: self.high_wqc.clone(),
                        round: timeout.round,
                        votes: self.votes.clone(),
                    }),
                ));
            }
        }

        Ok((self.weight, None))
    }
}
