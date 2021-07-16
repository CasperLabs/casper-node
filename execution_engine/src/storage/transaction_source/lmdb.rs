use std::path::Path;

use casper_types::bytesrepr::Bytes;
use lmdb::{
    self, Database, Environment, EnvironmentFlags, RoTransaction, RwTransaction, WriteFlags,
};

use crate::storage::{
    error,
    transaction_source::{Readable, Transaction, TransactionSource, Writable},
    MAX_DBS,
};

/// Filename for the LMDB database created by the EE.
const EE_DB_FILENAME: &str = "data.lmdb";

impl<'a> Transaction for RoTransaction<'a> {
    type Error = lmdb::Error;

    type Handle = Database;

    fn commit(self) -> Result<(), Self::Error> {
        lmdb::Transaction::commit(self)
    }
}

impl<'a> Readable for RoTransaction<'a> {
    fn read(&self, handle: Self::Handle, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        match lmdb::Transaction::get(self, handle, &key) {
            Ok(bytes) => Ok(Some(Bytes::from(bytes))),
            Err(lmdb::Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl<'a> Transaction for RwTransaction<'a> {
    type Error = lmdb::Error;

    type Handle = Database;

    fn commit(self) -> Result<(), Self::Error> {
        <RwTransaction<'a> as lmdb::Transaction>::commit(self)
    }
}

impl<'a> Readable for RwTransaction<'a> {
    fn read(&self, handle: Self::Handle, key: &[u8]) -> Result<Option<Bytes>, Self::Error> {
        match lmdb::Transaction::get(self, handle, &key) {
            Ok(bytes) => Ok(Some(Bytes::from(bytes))),
            Err(lmdb::Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl<'a> Writable for RwTransaction<'a> {
    fn write(&mut self, handle: Self::Handle, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.put(handle, &key, &value, WriteFlags::empty())
            .map_err(Into::into)
    }
}

/// The environment for an LMDB-backed trie store.
///
/// Wraps [`lmdb::Environment`].
#[derive(Debug)]
pub struct LmdbEnvironment {
    env: Environment,
}

impl LmdbEnvironment {
    pub fn new<P: AsRef<Path>>(
        path: P,
        map_size: usize,
        max_readers: u32,
    ) -> Result<Self, error::Error> {
        // These options mean we must manually call sync on the environment for the EE.
        let fast_options = EnvironmentFlags::MAP_ASYNC
            | EnvironmentFlags::WRITE_MAP
            | EnvironmentFlags::NO_META_SYNC;
        let env = Environment::new()
            // Set the flag to manage our own directory like in the storage component.
            .set_flags(EnvironmentFlags::NO_SUB_DIR | fast_options)
            .set_max_dbs(MAX_DBS)
            .set_map_size(map_size)
            .set_max_readers(max_readers)
            .open(&path.as_ref().join(EE_DB_FILENAME))?;
        Ok(LmdbEnvironment { env })
    }

    pub fn env(&self) -> &Environment {
        &self.env
    }
}

impl<'a> TransactionSource<'a> for LmdbEnvironment {
    type Error = lmdb::Error;

    type Handle = Database;

    type ReadTransaction = RoTransaction<'a>;

    type ReadWriteTransaction = RwTransaction<'a>;

    fn create_read_txn(&'a self) -> Result<RoTransaction<'a>, Self::Error> {
        self.env.begin_ro_txn()
    }

    fn create_read_write_txn(&'a self) -> Result<RwTransaction<'a>, Self::Error> {
        self.env.begin_rw_txn()
    }
}
