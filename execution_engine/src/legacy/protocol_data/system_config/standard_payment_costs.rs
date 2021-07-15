use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::system_config::standard_payment_costs::StandardPaymentCosts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacyStandardPaymentCosts {
    pay: u32,
}

impl From<LegacyStandardPaymentCosts> for StandardPaymentCosts {
    fn from(legacy_standard_payment_costs: LegacyStandardPaymentCosts) -> Self {
        StandardPaymentCosts {
            pay: legacy_standard_payment_costs.pay,
        }
    }
}

impl FromBytes for LegacyStandardPaymentCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (pay, rem) = FromBytes::from_bytes(bytes)?;
        let legacy_standard_payment_costs = LegacyStandardPaymentCosts { pay };
        Ok((legacy_standard_payment_costs, rem))
    }
}
