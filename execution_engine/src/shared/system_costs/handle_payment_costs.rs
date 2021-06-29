use casper_types::bytesrepr::{self, FromBytes, ToBytes};
use datasize::DataSize;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

pub const DEFAULT_GET_PAYMENT_PURSE_COST: u32 = 10_000;
pub const DEFAULT_SET_REFUND_PURSE_COST: u32 = 10_000;
pub const DEFAULT_GET_REFUND_PURSE_COST: u32 = 10_000;
pub const DEFAULT_FINALIZE_PAYMENT_COST: u32 = 10_000;

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
        let mut ret = bytesrepr::unchecked_allocate_buffer(self);

        ret.append(&mut self.get_payment_purse.to_bytes()?);
        ret.append(&mut self.set_refund_purse.to_bytes()?);
        ret.append(&mut self.get_refund_purse.to_bytes()?);
        ret.append(&mut self.finalize_payment.to_bytes()?);

        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.get_payment_purse.serialized_length()
            + self.set_refund_purse.serialized_length()
            + self.get_refund_purse.serialized_length()
            + self.finalize_payment.serialized_length()
    }
}

impl FromBytes for HandlePaymentCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
        let (get_payment_purse, rem) = FromBytes::from_bytes(bytes)?;
        let (set_refund_purse, rem) = FromBytes::from_bytes(rem)?;
        let (get_refund_purse, rem) = FromBytes::from_bytes(rem)?;
        let (finalize_payment, rem) = FromBytes::from_bytes(rem)?;

        Ok((
            Self {
                get_payment_purse,
                set_refund_purse,
                get_refund_purse,
                finalize_payment,
            },
            rem,
        ))
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
