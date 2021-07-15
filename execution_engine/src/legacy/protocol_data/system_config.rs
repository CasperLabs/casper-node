mod auction_costs;
mod handle_payment_costs;
mod mint_costs;
mod standard_payment_costs;

use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::system_config::SystemConfig;

use self::{
    auction_costs::LegacyAuctionCosts, handle_payment_costs::LegacyHandlePaymentCosts,
    mint_costs::LegacyMintCosts, standard_payment_costs::LegacyStandardPaymentCosts,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacySystemConfig {
    wasmless_transfer_cost: u32,
    auction_costs: LegacyAuctionCosts,
    mint_costs: LegacyMintCosts,
    handle_payment_costs: LegacyHandlePaymentCosts,
    standard_payment_costs: LegacyStandardPaymentCosts,
}

impl From<LegacySystemConfig> for SystemConfig {
    fn from(legacy_system_config: LegacySystemConfig) -> Self {
        SystemConfig::new(
            legacy_system_config.wasmless_transfer_cost,
            legacy_system_config.auction_costs.into(),
            legacy_system_config.mint_costs.into(),
            legacy_system_config.handle_payment_costs.into(),
            legacy_system_config.standard_payment_costs.into(),
        )
    }
}

impl FromBytes for LegacySystemConfig {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (wasmless_transfer_cost, rem) = FromBytes::from_bytes(bytes)?;
        let (auction_costs, rem) = FromBytes::from_bytes(rem)?;
        let (mint_costs, rem) = FromBytes::from_bytes(rem)?;
        let (handle_payment_costs, rem) = FromBytes::from_bytes(rem)?;
        let (standard_payment_costs, rem) = FromBytes::from_bytes(rem)?;

        let legacy_system_config = LegacySystemConfig {
            wasmless_transfer_cost,
            auction_costs,
            mint_costs,
            handle_payment_costs,
            standard_payment_costs,
        };

        Ok((legacy_system_config, rem))
    }
}
