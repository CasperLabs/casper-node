//! Serialization and deserialization of legacy types.
use casper_types::{
    bytesrepr::{self, FromBytes},
    ContractHash,
};

/// An alias for legacy [`Key`]s hash variant.
#[derive(Copy, Clone, Debug)]
pub struct LegacyHashAddr([u8; 32]);

impl From<LegacyHashAddr> for ContractHash {
    fn from(legacy_hash_addr: LegacyHashAddr) -> Self {
        ContractHash::new(legacy_hash_addr.0)
    }
}

impl FromBytes for LegacyHashAddr {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (bytes, rem) = FromBytes::from_bytes(bytes)?;
        Ok((LegacyHashAddr(bytes), rem))
    }
}
