use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::storage_costs::StorageCosts;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct LegacyStorageCosts {
    gas_per_byte: u32,
}

impl From<LegacyStorageCosts> for StorageCosts {
    fn from(legacy_storage_costs: LegacyStorageCosts) -> Self {
        StorageCosts::new(legacy_storage_costs.gas_per_byte)
    }
}

impl FromBytes for LegacyStorageCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (gas_per_byte, rem) = FromBytes::from_bytes(bytes)?;

        let legacy_storage_costs = LegacyStorageCosts { gas_per_byte };

        Ok((legacy_storage_costs, rem))
    }
}
