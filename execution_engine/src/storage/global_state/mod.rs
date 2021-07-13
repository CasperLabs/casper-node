pub mod in_memory;
pub mod lmdb;

use std::hash::BuildHasher;

use tracing::error;

use casper_types::{bytesrepr, Key, ProtocolVersion};

use crate::{
    shared::{
        additive_map::AdditiveMap,
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
        transform::{self, Transform},
    },
    storage::{
        protocol_data::ProtocolData,
        transaction_source::{Transaction, TransactionSource},
        trie::{merkle_proof::TrieMerkleProof, Trie},
        trie_store::{
            operations::{read, write, ReadResult, WriteResult},
            TrieStore,
        },
    },
};

/// A reader of state
pub trait StateReader<K, V> {
    /// An error which occurs when reading state
    type Error;

    /// Returns the state value from the corresponding key
    fn read(&self, correlation_id: CorrelationId, key: &K) -> Result<Option<V>, Self::Error>;

    /// Returns the merkle proof of the state value from the corresponding key
    fn read_with_proof(
        &self,
        correlation_id: CorrelationId,
        key: &K,
    ) -> Result<Option<TrieMerkleProof<K, V>>, Self::Error>;

    /// Returns the keys in the trie matching `prefix`.
    fn keys_with_prefix(
        &self,
        correlation_id: CorrelationId,
        prefix: &[u8],
    ) -> Result<Vec<K>, Self::Error>;
}

/// An error emitted by the execution engine on commit
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CommitError {
    #[error("Root not found: {0:?}")]
    RootNotFound(Blake2bHash),
    #[error("Root not found while attempting to read: {0:?}")]
    ReadRootNotFound(Blake2bHash),
    #[error("Root not found while writing: {0:?}")]
    WriteRootNotFound(Blake2bHash),
    #[error("Key not found: {0}")]
    KeyNotFound(Key),
    #[error(transparent)]
    TransformError(transform::Error),
}

pub trait StateProvider {
    type Error;
    type Reader: StateReader<Key, StoredValue, Error = Self::Error>;

    /// Checkouts to the post state of a specific block.
    fn checkout(&self, state_hash: Blake2bHash) -> Result<Option<Self::Reader>, Self::Error>;

    /// Applies changes and returns a new post state hash.
    /// block_hash is used for computing a deterministic and unique keys.
    fn commit(
        &self,
        correlation_id: CorrelationId,
        state_hash: Blake2bHash,
        effects: AdditiveMap<Key, Transform>,
    ) -> Result<Blake2bHash, Self::Error>;

    fn put_protocol_data(
        &self,
        protocol_version: ProtocolVersion,
        protocol_data: &ProtocolData,
    ) -> Result<(), Self::Error>;

    fn get_protocol_data(
        &self,
        protocol_version: ProtocolVersion,
    ) -> Result<Option<ProtocolData>, Self::Error>;

    fn empty_root(&self) -> Blake2bHash;

    /// Reads a `Trie` from the state if it is present
    fn read_trie(
        &self,
        correlation_id: CorrelationId,
        trie_key: &Blake2bHash,
    ) -> Result<Option<Trie<Key, StoredValue>>, Self::Error>;

    /// Insert a trie node into the trie
    fn put_trie(
        &self,
        correlation_id: CorrelationId,
        trie: &Trie<Key, StoredValue>,
    ) -> Result<Blake2bHash, Self::Error>;

    /// Finds all of the missing or corrupt keys of which are descendants of `trie_key`
    fn missing_trie_keys(
        &self,
        correlation_id: CorrelationId,
        trie_keys: Vec<Blake2bHash>,
    ) -> Result<Vec<Blake2bHash>, Self::Error>;
}

pub fn commit<'a, R, S, H, E>(
    environment: &'a R,
    store: &S,
    correlation_id: CorrelationId,
    prestate_hash: Blake2bHash,
    effects: AdditiveMap<Key, Transform, H>,
) -> Result<Blake2bHash, E>
where
    R: TransactionSource<'a, Handle = S::Handle>,
    S: TrieStore<Key, StoredValue>,
    S::Error: From<R::Error>,
    E: From<R::Error> + From<S::Error> + From<bytesrepr::Error> + From<CommitError>,
    H: BuildHasher,
{
    let mut txn = environment.create_read_write_txn()?;
    let mut state_root = prestate_hash;

    let maybe_root: Option<Trie<Key, StoredValue>> = store.get(&txn, &state_root)?;

    if maybe_root.is_none() {
        return Err(CommitError::RootNotFound(prestate_hash).into());
    };

    for (key, transform) in effects.into_iter() {
        let read_result = read::<_, _, _, _, E>(correlation_id, &txn, store, &state_root, &key)?;

        let value = match (read_result, transform) {
            (ReadResult::NotFound, Transform::Write(new_value)) => new_value,
            (ReadResult::NotFound, transform) => {
                error!(
                    ?state_root,
                    ?key,
                    ?transform,
                    "Key not found while attempting to apply transform"
                );
                return Err(CommitError::KeyNotFound(key).into());
            }
            (ReadResult::Found(current_value), transform) => match transform.apply(current_value) {
                Ok(updated_value) => updated_value,
                Err(err) => {
                    error!(
                        ?state_root,
                        ?key,
                        ?err,
                        "Key found, but could not apply transform"
                    );
                    return Err(CommitError::TransformError(err).into());
                }
            },
            (ReadResult::RootNotFound, transform) => {
                error!(
                    ?state_root,
                    ?key,
                    ?transform,
                    "Failed to read state root while processing transform"
                );
                return Err(CommitError::ReadRootNotFound(state_root).into());
            }
        };

        let write_result =
            write::<_, _, _, _, E>(correlation_id, &mut txn, store, &state_root, &key, &value)?;

        match write_result {
            WriteResult::Written(root_hash) => {
                state_root = root_hash;
            }
            WriteResult::AlreadyExists => (),
            WriteResult::RootNotFound => {
                error!(?state_root, ?key, ?value, "Error writing new value");
                return Err(CommitError::WriteRootNotFound(state_root).into());
            }
        }
    }

    txn.commit()?;

    Ok(state_root)
}
