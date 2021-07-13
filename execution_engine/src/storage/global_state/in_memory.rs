use std::{ops::Deref, sync::Arc};

use crate::shared::{
    additive_map::AdditiveMap,
    newtypes::{Blake2bHash, CorrelationId},
    stored_value::StoredValue,
    transform::Transform,
};
use casper_types::{Key, ProtocolVersion};

use crate::storage::{
    error::{self, in_memory},
    global_state::{commit, StateProvider, StateReader},
    protocol_data::ProtocolData,
    protocol_data_store::in_memory::InMemoryProtocolDataStore,
    store::Store,
    transaction_source::{
        in_memory::{InMemoryEnvironment, InMemoryReadTransaction, InMemoryReadWriteTransaction},
        Transaction, TransactionSource,
    },
    trie::{merkle_proof::TrieMerkleProof, operations::create_hashed_empty_trie, Trie},
    trie_store::{
        in_memory::InMemoryTrieStore,
        operations::{
            self, keys_with_prefix, missing_trie_keys, put_trie, read, read_with_proof, ReadResult,
            WriteResult,
        },
    },
};

pub struct InMemoryGlobalState {
    pub environment: Arc<InMemoryEnvironment>,
    pub trie_store: Arc<InMemoryTrieStore>,
    pub protocol_data_store: Arc<InMemoryProtocolDataStore>,
    pub empty_root_hash: Blake2bHash,
}

/// Represents a "view" of global state at a particular root hash.
pub struct InMemoryGlobalStateView {
    pub environment: Arc<InMemoryEnvironment>,
    pub store: Arc<InMemoryTrieStore>,
    pub root_hash: Blake2bHash,
}

impl InMemoryGlobalState {
    /// Creates an empty state.
    pub fn empty() -> Result<Self, error::Error> {
        let environment = Arc::new(InMemoryEnvironment::new());
        let trie_store = Arc::new(InMemoryTrieStore::new(&environment, None));
        let protocol_data_store = Arc::new(InMemoryProtocolDataStore::new(&environment, None));
        let root_hash: Blake2bHash = {
            let (root_hash, root) = create_hashed_empty_trie::<Key, StoredValue>()?;
            let mut txn = environment.create_read_write_txn()?;
            trie_store.put(&mut txn, &root_hash, &root)?;
            txn.commit()?;
            root_hash
        };
        Ok(InMemoryGlobalState::new(
            environment,
            trie_store,
            protocol_data_store,
            root_hash,
        ))
    }

    /// Creates a state from an existing environment, trie_Store, and root_hash.
    /// Intended to be used for testing.
    pub(crate) fn new(
        environment: Arc<InMemoryEnvironment>,
        trie_store: Arc<InMemoryTrieStore>,
        protocol_data_store: Arc<InMemoryProtocolDataStore>,
        empty_root_hash: Blake2bHash,
    ) -> Self {
        InMemoryGlobalState {
            environment,
            trie_store,
            protocol_data_store,
            empty_root_hash,
        }
    }

    /// Creates a state from a given set of `Key, StoredValue` pairs.
    pub fn from_pairs(
        correlation_id: CorrelationId,
        pairs: &[(Key, StoredValue)],
    ) -> Result<(Self, Blake2bHash), error::Error> {
        let state = InMemoryGlobalState::empty()?;
        let mut current_root = state.empty_root_hash;
        {
            let mut txn = state.environment.create_read_write_txn()?;
            for (key, value) in pairs {
                let key = key.normalize();
                match operations::write::<_, _, _, InMemoryTrieStore, in_memory::Error>(
                    correlation_id,
                    &mut txn,
                    &state.trie_store,
                    &current_root,
                    &key,
                    value,
                )? {
                    WriteResult::Written(root_hash) => {
                        current_root = root_hash;
                    }
                    WriteResult::AlreadyExists => (),
                    WriteResult::RootNotFound => panic!("InMemoryGlobalState has invalid root"),
                }
            }
            txn.commit()?;
        }
        Ok((state, current_root))
    }
}

impl StateReader<Key, StoredValue> for InMemoryGlobalStateView {
    type Error = error::Error;

    fn read(
        &self,
        correlation_id: CorrelationId,
        key: &Key,
    ) -> Result<Option<StoredValue>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let ret = match read::<
            Key,
            StoredValue,
            InMemoryReadTransaction,
            InMemoryTrieStore,
            Self::Error,
        >(
            correlation_id,
            &txn,
            self.store.deref(),
            &self.root_hash,
            key,
        )? {
            ReadResult::Found(value) => Some(value),
            ReadResult::NotFound => None,
            ReadResult::RootNotFound => panic!("InMemoryGlobalState has invalid root"),
        };
        txn.commit()?;
        Ok(ret)
    }

    fn read_with_proof(
        &self,
        correlation_id: CorrelationId,
        key: &Key,
    ) -> Result<Option<TrieMerkleProof<Key, StoredValue>>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let ret = match read_with_proof::<
            Key,
            StoredValue,
            InMemoryReadTransaction,
            InMemoryTrieStore,
            Self::Error,
        >(
            correlation_id,
            &txn,
            self.store.deref(),
            &self.root_hash,
            key,
        )? {
            ReadResult::Found(value) => Some(value),
            ReadResult::NotFound => None,
            ReadResult::RootNotFound => panic!("InMemoryGlobalState has invalid root"),
        };
        txn.commit()?;
        Ok(ret)
    }

    fn keys_with_prefix(
        &self,
        correlation_id: CorrelationId,
        prefix: &[u8],
    ) -> Result<Vec<Key>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let keys_iter = keys_with_prefix::<Key, StoredValue, _, _>(
            correlation_id,
            &txn,
            self.store.deref(),
            &self.root_hash,
            prefix,
        );
        let mut ret: Vec<Key> = Vec::new();
        for result in keys_iter {
            match result {
                Ok(key) => ret.push(key),
                Err(error) => return Err(error.into()),
            }
        }
        txn.commit()?;
        Ok(ret)
    }
}

impl StateProvider for InMemoryGlobalState {
    type Error = error::Error;

    type Reader = InMemoryGlobalStateView;

    fn checkout(&self, prestate_hash: Blake2bHash) -> Result<Option<Self::Reader>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let maybe_root: Option<Trie<Key, StoredValue>> =
            self.trie_store.get(&txn, &prestate_hash)?;
        let maybe_state = maybe_root.map(|_| InMemoryGlobalStateView {
            environment: Arc::clone(&self.environment),
            store: Arc::clone(&self.trie_store),
            root_hash: prestate_hash,
        });
        txn.commit()?;
        Ok(maybe_state)
    }

    fn commit(
        &self,
        correlation_id: CorrelationId,
        prestate_hash: Blake2bHash,
        effects: AdditiveMap<Key, Transform>,
    ) -> Result<Blake2bHash, Self::Error> {
        commit::<InMemoryEnvironment, InMemoryTrieStore, _, Self::Error>(
            &self.environment,
            &self.trie_store,
            correlation_id,
            prestate_hash,
            effects,
        )
        .map_err(Into::into)
    }

    fn put_protocol_data(
        &self,
        protocol_version: ProtocolVersion,
        protocol_data: &ProtocolData,
    ) -> Result<(), Self::Error> {
        let mut txn = self.environment.create_read_write_txn()?;
        self.protocol_data_store
            .put(&mut txn, &protocol_version, protocol_data)?;
        txn.commit().map_err(Into::into)
    }

    fn get_protocol_data(
        &self,
        protocol_version: ProtocolVersion,
    ) -> Result<Option<ProtocolData>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let result = self.protocol_data_store.get(&txn, &protocol_version)?;
        txn.commit()?;
        Ok(result)
    }

    fn empty_root(&self) -> Blake2bHash {
        self.empty_root_hash
    }

    fn read_trie(
        &self,
        _correlation_id: CorrelationId,
        trie_key: &Blake2bHash,
    ) -> Result<Option<Trie<Key, StoredValue>>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let ret: Option<Trie<Key, StoredValue>> = self.trie_store.get(&txn, trie_key)?;
        txn.commit()?;
        Ok(ret)
    }

    fn put_trie(
        &self,
        correlation_id: CorrelationId,
        trie: &Trie<Key, StoredValue>,
    ) -> Result<Blake2bHash, Self::Error> {
        let mut txn = self.environment.create_read_write_txn()?;
        let trie_hash = put_trie::<
            Key,
            StoredValue,
            InMemoryReadWriteTransaction,
            InMemoryTrieStore,
            Self::Error,
        >(correlation_id, &mut txn, &self.trie_store, trie)?;
        txn.commit()?;
        Ok(trie_hash)
    }

    /// Finds all of the keys of missing descendant `Trie<Key,StoredValue>` values
    fn missing_trie_keys(
        &self,
        correlation_id: CorrelationId,
        trie_keys: Vec<Blake2bHash>,
    ) -> Result<Vec<Blake2bHash>, Self::Error> {
        let txn = self.environment.create_read_txn()?;
        let missing_descendants =
            missing_trie_keys::<
                Key,
                StoredValue,
                InMemoryReadTransaction,
                InMemoryTrieStore,
                Self::Error,
            >(correlation_id, &txn, self.trie_store.deref(), trie_keys)?;
        txn.commit()?;
        Ok(missing_descendants)
    }
}

#[cfg(test)]
mod tests {
    use crate::shared::newtypes::Blake2bHash;
    use casper_types::{account::AccountHash, CLValue};

    use super::*;

    #[derive(Debug, Clone)]
    struct TestPair {
        key: Key,
        value: StoredValue,
    }

    fn create_test_pairs() -> [TestPair; 2] {
        [
            TestPair {
                key: Key::Account(AccountHash::new([1_u8; 32])),
                value: StoredValue::CLValue(CLValue::from_t(1_i32).unwrap()),
            },
            TestPair {
                key: Key::Account(AccountHash::new([2_u8; 32])),
                value: StoredValue::CLValue(CLValue::from_t(2_i32).unwrap()),
            },
        ]
    }

    fn create_test_pairs_updated() -> [TestPair; 3] {
        [
            TestPair {
                key: Key::Account(AccountHash::new([1u8; 32])),
                value: StoredValue::CLValue(CLValue::from_t("one".to_string()).unwrap()),
            },
            TestPair {
                key: Key::Account(AccountHash::new([2u8; 32])),
                value: StoredValue::CLValue(CLValue::from_t("two".to_string()).unwrap()),
            },
            TestPair {
                key: Key::Account(AccountHash::new([3u8; 32])),
                value: StoredValue::CLValue(CLValue::from_t(3_i32).unwrap()),
            },
        ]
    }

    fn create_test_state() -> (InMemoryGlobalState, Blake2bHash) {
        InMemoryGlobalState::from_pairs(
            CorrelationId::new(),
            &create_test_pairs()
                .iter()
                .cloned()
                .map(|TestPair { key, value }| (key, value))
                .collect::<Vec<(Key, StoredValue)>>(),
        )
        .unwrap()
    }

    #[test]
    fn reads_from_a_checkout_return_expected_values() {
        let correlation_id = CorrelationId::new();
        let (state, root_hash) = create_test_state();
        let checkout = state.checkout(root_hash).unwrap().unwrap();
        for TestPair { key, value } in create_test_pairs().iter().cloned() {
            assert_eq!(Some(value), checkout.read(correlation_id, &key).unwrap());
        }
    }

    #[test]
    fn checkout_fails_if_unknown_hash_is_given() {
        let (state, _) = create_test_state();
        let fake_hash = Blake2bHash::new(&[1, 2, 3]);
        let result = state.checkout(fake_hash).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn commit_updates_state() {
        let correlation_id = CorrelationId::new();

        let test_pairs_updated = create_test_pairs_updated();

        let (state, root_hash) = create_test_state();

        let effects: AdditiveMap<Key, Transform> = test_pairs_updated
            .iter()
            .cloned()
            .map(|TestPair { key, value }| (key, Transform::Write(value)))
            .collect();

        let updated_hash = state.commit(correlation_id, root_hash, effects).unwrap();

        let updated_checkout = state.checkout(updated_hash).unwrap().unwrap();

        for TestPair { key, value } in test_pairs_updated.iter().cloned() {
            assert_eq!(
                Some(value),
                updated_checkout.read(correlation_id, &key).unwrap()
            );
        }
    }

    #[test]
    fn commit_updates_state_and_original_state_stays_intact() {
        let correlation_id = CorrelationId::new();
        let test_pairs_updated = create_test_pairs_updated();

        let (state, root_hash) = create_test_state();

        let effects: AdditiveMap<Key, Transform> = {
            let mut tmp = AdditiveMap::new();
            for TestPair { key, value } in &test_pairs_updated {
                tmp.insert(*key, Transform::Write(value.to_owned()));
            }
            tmp
        };

        let updated_hash = state.commit(correlation_id, root_hash, effects).unwrap();

        let updated_checkout = state.checkout(updated_hash).unwrap().unwrap();
        for TestPair { key, value } in test_pairs_updated.iter().cloned() {
            assert_eq!(
                Some(value),
                updated_checkout.read(correlation_id, &key).unwrap()
            );
        }

        let original_checkout = state.checkout(root_hash).unwrap().unwrap();
        for TestPair { key, value } in create_test_pairs().iter().cloned() {
            assert_eq!(
                Some(value),
                original_checkout.read(correlation_id, &key).unwrap()
            );
        }
        assert_eq!(
            None,
            original_checkout
                .read(correlation_id, &test_pairs_updated[2].key)
                .unwrap()
        );
    }

    #[test]
    fn initial_state_has_the_expected_hash() {
        let correlation_id = CorrelationId::new();
        let expected_bytes = vec![
            197, 117, 38, 12, 241, 62, 54, 241, 121, 165, 11, 8, 130, 189, 100, 252, 4, 102, 236,
            210, 91, 221, 123, 200, 135, 102, 194, 204, 46, 76, 13, 254,
        ];
        let (_, root_hash) = InMemoryGlobalState::from_pairs(correlation_id, &[]).unwrap();
        assert_eq!(expected_bytes, root_hash.to_vec())
    }
}
