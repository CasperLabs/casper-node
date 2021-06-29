use casper_types::bytesrepr::{self, FromBytes, ToBytes};
use datasize::DataSize;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

pub const DEFAULT_GET_ERA_VALIDATORS_COST: u32 = 10_000;
pub const DEFAULT_READ_SEIGNIORAGE_RECIPIENTS_COST: u32 = 10_000;
pub const DEFAULT_ADD_BID_COST: u32 = 10_000;
pub const DEFAULT_WITHDRAW_BID_COST: u32 = 10_000;
pub const DEFAULT_DELEGATE_COST: u32 = 10_000;
pub const DEFAULT_UNDELEGATE_COST: u32 = 10_000;
pub const DEFAULT_RUN_AUCTION_COST: u32 = 10_000;
pub const DEFAULT_SLASH_COST: u32 = 10_000;
pub const DEFAULT_DISTRIBUTE_COST: u32 = 10_000;
pub const DEFAULT_WITHDRAW_DELEGATOR_REWARD_COST: u32 = 10_000;
pub const DEFAULT_WITHDRAW_VALIDATOR_REWARD_COST: u32 = 10_000;
pub const DEFAULT_READ_ERA_ID_COST: u32 = 10_000;
pub const DEFAULT_ACTIVATE_BID_COST: u32 = 10_000;

/// Description of costs of calling auction entrypoints.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct AuctionCosts {
    pub get_era_validators: u32,
    pub read_seigniorage_recipients: u32,
    pub add_bid: u32,
    pub withdraw_bid: u32,
    pub delegate: u32,
    pub undelegate: u32,
    pub run_auction: u32,
    pub slash: u32,
    pub distribute: u32,
    pub withdraw_delegator_reward: u32,
    pub withdraw_validator_reward: u32,
    pub read_era_id: u32,
    pub activate_bid: u32,
}

impl Default for AuctionCosts {
    fn default() -> Self {
        Self {
            get_era_validators: DEFAULT_GET_ERA_VALIDATORS_COST,
            read_seigniorage_recipients: DEFAULT_READ_SEIGNIORAGE_RECIPIENTS_COST,
            add_bid: DEFAULT_ADD_BID_COST,
            withdraw_bid: DEFAULT_WITHDRAW_BID_COST,
            delegate: DEFAULT_DELEGATE_COST,
            undelegate: DEFAULT_UNDELEGATE_COST,
            run_auction: DEFAULT_RUN_AUCTION_COST,
            slash: DEFAULT_SLASH_COST,
            distribute: DEFAULT_DISTRIBUTE_COST,
            withdraw_delegator_reward: DEFAULT_WITHDRAW_DELEGATOR_REWARD_COST,
            withdraw_validator_reward: DEFAULT_WITHDRAW_VALIDATOR_REWARD_COST,
            read_era_id: DEFAULT_READ_ERA_ID_COST,
            activate_bid: DEFAULT_ACTIVATE_BID_COST,
        }
    }
}

impl ToBytes for AuctionCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, casper_types::bytesrepr::Error> {
        let mut ret = bytesrepr::unchecked_allocate_buffer(self);

        ret.append(&mut self.get_era_validators.to_bytes()?);
        ret.append(&mut self.read_seigniorage_recipients.to_bytes()?);
        ret.append(&mut self.add_bid.to_bytes()?);
        ret.append(&mut self.withdraw_bid.to_bytes()?);
        ret.append(&mut self.delegate.to_bytes()?);
        ret.append(&mut self.undelegate.to_bytes()?);
        ret.append(&mut self.run_auction.to_bytes()?);
        ret.append(&mut self.slash.to_bytes()?);
        ret.append(&mut self.distribute.to_bytes()?);
        ret.append(&mut self.withdraw_delegator_reward.to_bytes()?);
        ret.append(&mut self.withdraw_validator_reward.to_bytes()?);
        ret.append(&mut self.read_era_id.to_bytes()?);
        ret.append(&mut self.activate_bid.to_bytes()?);

        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.get_era_validators.serialized_length()
            + self.read_seigniorage_recipients.serialized_length()
            + self.add_bid.serialized_length()
            + self.withdraw_bid.serialized_length()
            + self.delegate.serialized_length()
            + self.undelegate.serialized_length()
            + self.run_auction.serialized_length()
            + self.slash.serialized_length()
            + self.distribute.serialized_length()
            + self.withdraw_delegator_reward.serialized_length()
            + self.withdraw_validator_reward.serialized_length()
            + self.read_era_id.serialized_length()
            + self.activate_bid.serialized_length()
    }
}

impl FromBytes for AuctionCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
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
        Ok((
            Self {
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
            },
            rem,
        ))
    }
}

impl Distribution<AuctionCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AuctionCosts {
        AuctionCosts {
            get_era_validators: rng.gen(),
            read_seigniorage_recipients: rng.gen(),
            add_bid: rng.gen(),
            withdraw_bid: rng.gen(),
            delegate: rng.gen(),
            undelegate: rng.gen(),
            run_auction: rng.gen(),
            slash: rng.gen(),
            distribute: rng.gen(),
            withdraw_delegator_reward: rng.gen(),
            withdraw_validator_reward: rng.gen(),
            read_era_id: rng.gen(),
            activate_bid: rng.gen(),
        }
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::AuctionCosts;

    prop_compose! {
        pub fn auction_costs_arb()(
            get_era_validators in num::u32::ANY,
            read_seigniorage_recipients in num::u32::ANY,
            add_bid in num::u32::ANY,
            withdraw_bid in num::u32::ANY,
            delegate in num::u32::ANY,
            undelegate in num::u32::ANY,
            run_auction in num::u32::ANY,
            slash in num::u32::ANY,
            distribute in num::u32::ANY,
            withdraw_delegator_reward in num::u32::ANY,
            withdraw_validator_reward in num::u32::ANY,
            read_era_id in num::u32::ANY,
            activate_bid in num::u32::ANY,
        ) -> AuctionCosts {
            AuctionCosts {
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
            }
        }
    }
}
