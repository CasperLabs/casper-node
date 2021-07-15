use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::system_config::auction_costs::AuctionCosts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacyAuctionCosts {
    get_era_validators: u32,
    read_seigniorage_recipients: u32,
    add_bid: u32,
    withdraw_bid: u32,
    delegate: u32,
    undelegate: u32,
    run_auction: u32,
    slash: u32,
    distribute: u32,
    withdraw_delegator_reward: u32,
    withdraw_validator_reward: u32,
    read_era_id: u32,
    activate_bid: u32,
}

impl From<LegacyAuctionCosts> for AuctionCosts {
    fn from(legacy_auction_costs: LegacyAuctionCosts) -> Self {
        AuctionCosts {
            get_era_validators: legacy_auction_costs.get_era_validators,
            read_seigniorage_recipients: legacy_auction_costs.read_seigniorage_recipients,
            add_bid: legacy_auction_costs.add_bid,
            withdraw_bid: legacy_auction_costs.withdraw_bid,
            delegate: legacy_auction_costs.delegate,
            undelegate: legacy_auction_costs.undelegate,
            run_auction: legacy_auction_costs.run_auction,
            slash: legacy_auction_costs.slash,
            distribute: legacy_auction_costs.distribute,
            withdraw_delegator_reward: legacy_auction_costs.withdraw_delegator_reward,
            withdraw_validator_reward: legacy_auction_costs.withdraw_validator_reward,
            read_era_id: legacy_auction_costs.read_era_id,
            activate_bid: legacy_auction_costs.activate_bid,
        }
    }
}

impl FromBytes for LegacyAuctionCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (get_era_validators, rem) = FromBytes::from_bytes(bytes)?;
        let (read_seigniorage_recipients, rem) = FromBytes::from_bytes(rem)?;
        let (add_bid, rem) = FromBytes::from_bytes(rem)?;
        let (withdraw_bid, rem) = FromBytes::from_bytes(rem)?;
        let (delegate, rem) = FromBytes::from_bytes(rem)?;
        let (undelegate, rem) = FromBytes::from_bytes(rem)?;
        let (run_auction, rem) = FromBytes::from_bytes(rem)?;
        let (slash, rem) = FromBytes::from_bytes(rem)?;
        let (distribute, rem) = FromBytes::from_bytes(rem)?;
        let (withdraw_delegator_reward, rem) = FromBytes::from_bytes(rem)?;
        let (withdraw_validator_reward, rem) = FromBytes::from_bytes(rem)?;
        let (read_era_id, rem) = FromBytes::from_bytes(rem)?;
        let (activate_bid, rem) = FromBytes::from_bytes(rem)?;

        let legacy_auction_costs = LegacyAuctionCosts {
            get_era_validators,
            read_seigniorage_recipients,
            add_bid,
            withdraw_bid,
            delegate,
            undelegate,
            run_auction,
            slash,
            distribute,
            withdraw_delegator_reward,
            withdraw_validator_reward,
            read_era_id,
            activate_bid,
        };

        Ok((legacy_auction_costs, rem))
    }
}
