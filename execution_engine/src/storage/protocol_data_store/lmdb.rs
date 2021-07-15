use casper_types::{bytesrepr, ProtocolVersion};
use lmdb::{Database, DatabaseFlags};

use crate::storage::{
    error,
    protocol_data::{self, ProtocolData},
    protocol_data_store::{self, ProtocolDataStore},
    store::Store,
    transaction_source::lmdb::LmdbEnvironment,
};

/// An LMDB-backed protocol data store.
///
/// Wraps [`lmdb::Database`].
#[derive(Debug, Clone)]
pub struct LmdbProtocolDataStore {
    db: Database,
}

impl LmdbProtocolDataStore {
    pub fn new(
        env: &LmdbEnvironment,
        maybe_name: Option<&str>,
        flags: DatabaseFlags,
    ) -> Result<Self, error::Error> {
        let name = Self::name(maybe_name);
        let db = env.env().create_db(Some(&name), flags)?;
        Ok(LmdbProtocolDataStore { db })
    }

    pub fn open(env: &LmdbEnvironment, maybe_name: Option<&str>) -> Result<Self, error::Error> {
        let name = Self::name(maybe_name);
        let db = env.env().open_db(Some(&name))?;
        Ok(LmdbProtocolDataStore { db })
    }

    fn name(maybe_name: Option<&str>) -> String {
        maybe_name
            .map(|name| format!("{}-{}", protocol_data_store::NAME, name))
            .unwrap_or_else(|| String::from(protocol_data_store::NAME))
    }
}

impl Store<ProtocolVersion, ProtocolData> for LmdbProtocolDataStore {
    type Error = error::Error;

    type Handle = Database;

    fn handle(&self) -> Self::Handle {
        self.db
    }

    fn deserialize_value(
        &self,
        protocol_version: &ProtocolVersion,
        bytes: Vec<u8>,
    ) -> Result<ProtocolData, bytesrepr::Error> {
        protocol_data::get_versioned_protocol_data(protocol_version, bytes)
    }
}

impl ProtocolDataStore for LmdbProtocolDataStore {}
