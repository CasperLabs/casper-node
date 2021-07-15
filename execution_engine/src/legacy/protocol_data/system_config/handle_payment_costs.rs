use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::system_config::handle_payment_costs::HandlePaymentCosts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacyHandlePaymentCosts {
    get_payment_purse: u32,
    set_refund_purse: u32,
    get_refund_purse: u32,
    finalize_payment: u32,
}

impl From<LegacyHandlePaymentCosts> for HandlePaymentCosts {
    fn from(legacy_handle_payment_costs: LegacyHandlePaymentCosts) -> Self {
        HandlePaymentCosts {
            get_payment_purse: legacy_handle_payment_costs.get_payment_purse,
            set_refund_purse: legacy_handle_payment_costs.set_refund_purse,
            get_refund_purse: legacy_handle_payment_costs.get_refund_purse,
            finalize_payment: legacy_handle_payment_costs.finalize_payment,
        }
    }
}

impl FromBytes for LegacyHandlePaymentCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (get_payment_purse, rem) = FromBytes::from_bytes(bytes)?;
        let (set_refund_purse, rem) = FromBytes::from_bytes(rem)?;
        let (get_refund_purse, rem) = FromBytes::from_bytes(rem)?;
        let (finalize_payment, rem) = FromBytes::from_bytes(rem)?;
        let legacy_handle_payment_costs = LegacyHandlePaymentCosts {
            get_payment_purse,
            set_refund_purse,
            get_refund_purse,
            finalize_payment,
        };

        Ok((legacy_handle_payment_costs, rem))
    }
}
