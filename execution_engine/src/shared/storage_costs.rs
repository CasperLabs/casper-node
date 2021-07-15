use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::{
    bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes},
    U512,
};

use super::gas::Gas;

pub const DEFAULT_GAS_PER_BYTE_COST: u32 = 625_000;

#[derive(FromPrimitive, ToPrimitive)]
enum StorageCostsKeys {
    GasPerByte = 100,
}

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct StorageCosts {
    /// Gas charged per byte stored in the global state.
    gas_per_byte: u32,
}

impl StorageCosts {
    pub const fn new(gas_per_byte: u32) -> Self {
        Self { gas_per_byte }
    }

    pub fn gas_per_byte(&self) -> u32 {
        self.gas_per_byte
    }

    /// Calculates gas cost for storing `bytes`.
    pub fn calculate_gas_cost(&self, bytes: usize) -> Gas {
        let value = U512::from(self.gas_per_byte) * U512::from(bytes);
        Gas::new(value)
    }
}

impl Default for StorageCosts {
    fn default() -> Self {
        Self {
            gas_per_byte: DEFAULT_GAS_PER_BYTE_COST,
        }
    }
}

impl Distribution<StorageCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> StorageCosts {
        StorageCosts {
            gas_per_byte: rng.gen(),
        }
    }
}

impl ToBytes for StorageCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut writer = StructWriter::new();
        writer.write_pair(StorageCostsKeys::GasPerByte, self.gas_per_byte)?;
        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[self.gas_per_byte.serialized_length()])
    }
}

impl FromBytes for StorageCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let mut reader = StructReader::new(bytes);

        let mut storage_costs = StorageCosts::default();

        while let Some(key) = reader.read_key()? {
            match StorageCostsKeys::from_u64(key) {
                Some(StorageCostsKeys::GasPerByte) => {
                    storage_costs.gas_per_byte = reader.read_value()?
                }
                None => reader.skip_value()?,
            }
        }
        Ok((storage_costs, reader.finish()))
    }
}

#[cfg(test)]
pub mod tests {
    use casper_types::U512;

    use super::*;

    const SMALL_WEIGHT: usize = 123456789;
    const LARGE_WEIGHT: usize = usize::max_value();

    #[test]
    fn roundtrip_test() {
        bytesrepr::test_serialization_roundtrip(&StorageCosts::default());
    }

    #[test]
    fn should_calculate_gas_cost() {
        let storage_costs = StorageCosts::default();

        let cost = storage_costs.calculate_gas_cost(SMALL_WEIGHT);

        let expected_cost = U512::from(DEFAULT_GAS_PER_BYTE_COST) * U512::from(SMALL_WEIGHT);
        assert_eq!(cost, Gas::new(expected_cost));
    }

    #[test]
    fn should_calculate_big_gas_cost() {
        let storage_costs = StorageCosts::default();

        let cost = storage_costs.calculate_gas_cost(LARGE_WEIGHT);

        let expected_cost = U512::from(DEFAULT_GAS_PER_BYTE_COST) * U512::from(LARGE_WEIGHT);
        assert_eq!(cost, Gas::new(expected_cost));
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::StorageCosts;

    prop_compose! {
        pub fn storage_costs_arb()(
            gas_per_byte in num::u32::ANY,
        ) -> StorageCosts {
            StorageCosts {
                gas_per_byte,
            }
        }
    }
}
