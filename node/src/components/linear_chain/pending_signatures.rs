use datasize::DataSize;
use itertools::Itertools;
use std::collections::HashMap;
use tracing::warn;

use super::signature::Signature;
use crate::types::BlockHash;
use casper_types::PublicKey;

/// The maximum number of finality signatures from a single validator we keep in memory while
/// waiting for their block.
const MAX_PENDING_FINALITY_SIGNATURES_PER_VALIDATOR: usize = 1000;

/// Finality signatures to be inserted in a block once it is available.
/// Keyed by public key of the creator to limit the maximum amount of pending signatures.
#[derive(DataSize, Debug, Default)]
pub(super) struct PendingSignatures {
    pending_finality_signatures: HashMap<PublicKey, HashMap<BlockHash, Signature>>,
}

impl PendingSignatures {
    pub(super) fn new() -> Self {
        PendingSignatures {
            pending_finality_signatures: HashMap::new(),
        }
    }

    // Checks if we have already enqueued that finality signature.
    pub(super) fn has_finality_signature(
        &self,
        creator: &PublicKey,
        block_hash: &BlockHash,
    ) -> bool {
        self.pending_finality_signatures
            .get(creator)
            .map_or(false, |sigs| sigs.contains_key(block_hash))
    }

    /// Returns signatures for `block_hash` that are still pending.
    pub(super) fn collect_pending(&mut self, block_hash: &BlockHash) -> Vec<Signature> {
        let pending_sigs = self
            .pending_finality_signatures
            .values_mut()
            .filter_map(|sigs| sigs.remove(&block_hash))
            .collect_vec();
        self.remove_empty_entries();
        pending_sigs
    }

    /// Adds finality signature to the pending collection.
    /// Returns `true` if it was added.
    pub(super) fn add(&mut self, signature: Signature) -> bool {
        let public_key = signature.public_key();
        let block_hash = signature.block_hash();
        let sigs = self
            .pending_finality_signatures
            .entry(public_key.clone())
            .or_default();
        // Limit the memory we use for storing unknown signatures from each validator.
        if sigs.len() >= MAX_PENDING_FINALITY_SIGNATURES_PER_VALIDATOR {
            warn!(
                %block_hash, %public_key,
                "received too many finality signatures for unknown blocks"
            );
            return false;
        }
        // Add the pending signature.
        sigs.insert(block_hash, signature);
        true
    }

    pub(super) fn remove(
        &mut self,
        public_key: &PublicKey,
        block_hash: &BlockHash,
    ) -> Option<Signature> {
        let validator_sigs = self.pending_finality_signatures.get_mut(public_key)?;
        let sig = validator_sigs.remove(&block_hash);
        self.remove_empty_entries();
        sig
    }

    /// Removes all entries for which there are no finality signatures.
    fn remove_empty_entries(&mut self) {
        self.pending_finality_signatures
            .retain(|_, sigs| !sigs.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crypto::generate_ed25519_keypair, testing::TestRng, types::FinalitySignature};
    use casper_types::EraId;

    use std::collections::BTreeMap;

    #[test]
    fn membership_test() {
        let mut rng = TestRng::new();
        let mut pending_sigs = PendingSignatures::new();
        let block_hash = BlockHash::random(&mut rng);
        let block_hash_other = BlockHash::random(&mut rng);
        let sig_a = FinalitySignature::random_for_block(block_hash, 0);
        let sig_b = FinalitySignature::random_for_block(block_hash_other, 0);
        let public_key = sig_a.public_key.clone();
        let public_key_other = sig_b.public_key;
        assert!(pending_sigs.add(Signature::External(Box::new(sig_a))));
        assert!(pending_sigs.has_finality_signature(&public_key, &block_hash));
        assert!(!pending_sigs.has_finality_signature(&public_key_other, &block_hash));
        assert!(!pending_sigs.has_finality_signature(&public_key, &block_hash_other));
    }

    #[test]
    fn collect_pending() {
        let mut rng = TestRng::new();
        let mut pending_sigs = PendingSignatures::new();
        let block_hash = BlockHash::random(&mut rng);
        let block_hash_other = BlockHash::random(&mut rng);
        let sig_a1 = FinalitySignature::random_for_block(block_hash, 0);
        let sig_a2 = FinalitySignature::random_for_block(block_hash, 0);
        let sig_b = FinalitySignature::random_for_block(block_hash_other, 0);
        assert!(pending_sigs.add(Signature::External(Box::new(sig_a1.clone()))));
        assert!(pending_sigs.add(Signature::External(Box::new(sig_a2.clone()))));
        assert!(pending_sigs.add(Signature::External(Box::new(sig_b))));
        let collected_sigs: BTreeMap<PublicKey, FinalitySignature> = pending_sigs
            .collect_pending(&block_hash)
            .into_iter()
            .map(|sig| (sig.public_key(), *sig.take()))
            .collect();
        let expected_sigs = vec![sig_a1.clone(), sig_a2.clone()]
            .into_iter()
            .map(|sig| (sig.public_key.clone(), sig))
            .collect();
        assert_eq!(collected_sigs, expected_sigs);
        assert!(
            !pending_sigs.has_finality_signature(&sig_a1.public_key, &sig_a1.block_hash),
            "collecting should remove the signature"
        );
        assert!(
            !pending_sigs.has_finality_signature(&sig_a2.public_key, &sig_a2.block_hash),
            "collecting should remove the signature"
        );
    }

    #[test]
    fn remove_signature() {
        let mut rng = TestRng::new();
        let mut pending_sigs = PendingSignatures::new();
        let block_hash = BlockHash::random(&mut rng);
        let sig = FinalitySignature::random_for_block(block_hash, 0);
        assert!(pending_sigs.add(Signature::External(Box::new(sig.clone()))));
        let removed_sig = pending_sigs.remove(&sig.public_key, &sig.block_hash);
        assert!(removed_sig.is_some());
        assert!(!pending_sigs.has_finality_signature(&sig.public_key, &sig.block_hash));
        assert!(pending_sigs
            .remove(&sig.public_key, &sig.block_hash)
            .is_none());
    }

    #[test]
    fn max_limit_respected() {
        let mut rng = TestRng::new();
        let mut pending_sigs = PendingSignatures::new();
        let (sec_key, pub_key) = generate_ed25519_keypair();
        let era_id = EraId::new(0);
        for _ in 0..MAX_PENDING_FINALITY_SIGNATURES_PER_VALIDATOR {
            let block_hash = BlockHash::random(&mut rng);
            let sig = FinalitySignature::new(block_hash, era_id, &sec_key, pub_key.clone());
            assert!(pending_sigs.add(Signature::External(Box::new(sig))));
        }
        let block_hash = BlockHash::random(&mut rng);
        let sig = FinalitySignature::new(block_hash, era_id, &sec_key, pub_key);
        assert!(!pending_sigs.add(Signature::External(Box::new(sig))));
    }
}
