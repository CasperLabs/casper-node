use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes};

pub const DEFAULT_GET_PAYMENT_PURSE_COST: u32 = 10_000;
pub const DEFAULT_SET_REFUND_PURSE_COST: u32 = 10_000;
pub const DEFAULT_GET_REFUND_PURSE_COST: u32 = 10_000;
pub const DEFAULT_FINALIZE_PAYMENT_COST: u32 = 10_000;

#[derive(FromPrimitive, ToPrimitive)]
enum HandlePaymentCostsKeys {
    GetPaymentPurse = 100,
    SetRefundPurse = 101,
    GetRefundPurse = 102,
    FinalizePayment = 103,
}

/// Description of costs of calling handle payment entrypoints.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct HandlePaymentCosts {
    pub get_payment_purse: u32,
    pub set_refund_purse: u32,
    pub get_refund_purse: u32,
    pub finalize_payment: u32,
}

impl Default for HandlePaymentCosts {
    fn default() -> Self {
        Self {
            get_payment_purse: DEFAULT_GET_PAYMENT_PURSE_COST,
            set_refund_purse: DEFAULT_SET_REFUND_PURSE_COST,
            get_refund_purse: DEFAULT_GET_REFUND_PURSE_COST,
            finalize_payment: DEFAULT_FINALIZE_PAYMENT_COST,
        }
    }
}

impl ToBytes for HandlePaymentCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, casper_types::bytesrepr::Error> {
        let mut writer = StructWriter::new();

        writer.write_pair(
            HandlePaymentCostsKeys::GetPaymentPurse,
            self.get_payment_purse,
        )?;
        writer.write_pair(
            HandlePaymentCostsKeys::SetRefundPurse,
            self.set_refund_purse,
        )?;
        writer.write_pair(
            HandlePaymentCostsKeys::GetRefundPurse,
            self.get_refund_purse,
        )?;
        writer.write_pair(
            HandlePaymentCostsKeys::FinalizePayment,
            self.finalize_payment,
        )?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[
            self.get_payment_purse.serialized_length(),
            self.set_refund_purse.serialized_length(),
            self.get_refund_purse.serialized_length(),
            self.finalize_payment.serialized_length(),
        ])
    }
}

impl FromBytes for HandlePaymentCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
        let mut handle_payment_costs = HandlePaymentCosts::default();

        let mut reader = StructReader::new(bytes);

        while let Some(key) = reader.read_key()? {
            match HandlePaymentCostsKeys::from_u64(key) {
                Some(HandlePaymentCostsKeys::GetPaymentPurse) => {
                    handle_payment_costs.get_payment_purse = reader.read_value()?
                }
                Some(HandlePaymentCostsKeys::SetRefundPurse) => {
                    handle_payment_costs.set_refund_purse = reader.read_value()?
                }
                Some(HandlePaymentCostsKeys::GetRefundPurse) => {
                    handle_payment_costs.get_refund_purse = reader.read_value()?
                }
                Some(HandlePaymentCostsKeys::FinalizePayment) => {
                    handle_payment_costs.finalize_payment = reader.read_value()?
                }
                None => reader.skip_value()?,
            }
        }

        Ok((handle_payment_costs, reader.finish()))
    }
}

impl Distribution<HandlePaymentCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> HandlePaymentCosts {
        HandlePaymentCosts {
            get_payment_purse: rng.gen(),
            set_refund_purse: rng.gen(),
            get_refund_purse: rng.gen(),
            finalize_payment: rng.gen(),
        }
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::HandlePaymentCosts;

    prop_compose! {
        pub fn handle_payment_costs_arb()(
            get_payment_purse in num::u32::ANY,
            set_refund_purse in num::u32::ANY,
            get_refund_purse in num::u32::ANY,
            finalize_payment in num::u32::ANY,
        ) -> HandlePaymentCosts {
            HandlePaymentCosts {
                get_payment_purse,
                set_refund_purse,
                get_refund_purse,
                finalize_payment,
            }
        }
    }
}
