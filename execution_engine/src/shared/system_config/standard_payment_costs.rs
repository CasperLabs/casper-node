use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes};

pub const DEFAULT_PAY_COST: u32 = 10_000;

#[derive(FromPrimitive, ToPrimitive)]
enum StandardPaymentCostsKeys {
    Pay = 100,
}

/// Description of costs of calling standard payment entrypoints.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct StandardPaymentCosts {
    pub pay: u32,
}

impl Default for StandardPaymentCosts {
    fn default() -> Self {
        Self {
            pay: DEFAULT_PAY_COST,
        }
    }
}

impl ToBytes for StandardPaymentCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, casper_types::bytesrepr::Error> {
        let mut writer = StructWriter::new();

        writer.write_pair(StandardPaymentCostsKeys::Pay, self.pay)?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[self.pay.serialized_length()])
    }
}

impl FromBytes for StandardPaymentCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
        let mut standard_payment_costs = StandardPaymentCosts::default();

        let mut reader = StructReader::new(bytes);

        while let Some(key) = reader.read_key()? {
            match StandardPaymentCostsKeys::from_u64(key) {
                Some(StandardPaymentCostsKeys::Pay) => {
                    standard_payment_costs.pay = reader.read_value()?
                }
                None => reader.skip_value()?,
            }
        }

        Ok((standard_payment_costs, reader.finish()))
    }
}

impl Distribution<StandardPaymentCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> StandardPaymentCosts {
        StandardPaymentCosts { pay: rng.gen() }
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::StandardPaymentCosts;

    prop_compose! {
        pub fn standard_payment_costs_arb()(
            pay in num::u32::ANY,
        ) -> StandardPaymentCosts {
            StandardPaymentCosts {
                pay,
            }
        }
    }
}
