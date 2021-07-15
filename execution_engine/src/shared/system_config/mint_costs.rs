use casper_types::bytesrepr::{self, FromBytes, ToBytes};
use datasize::DataSize;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MINT_COST: u32 = 2_500_000_000;
pub const DEFAULT_REDUCE_TOTAL_SUPPLY_COST: u32 = 10_000;
pub const DEFAULT_CREATE_COST: u32 = 2_500_000_000;
pub const DEFAULT_BALANCE_COST: u32 = 10_000;
pub const DEFAULT_TRANSFER_COST: u32 = 10_000;
pub const DEFAULT_READ_BASE_ROUND_REWARD_COST: u32 = 10_000;

/// Description of costs of calling mint entrypoints.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct MintCosts {
    pub mint: u32,
    pub reduce_total_supply: u32,
    pub create: u32,
    pub balance: u32,
    pub transfer: u32,
    pub read_base_round_reward: u32,
}

impl Default for MintCosts {
    fn default() -> Self {
        Self {
            mint: DEFAULT_MINT_COST,
            reduce_total_supply: DEFAULT_REDUCE_TOTAL_SUPPLY_COST,
            create: DEFAULT_CREATE_COST,
            balance: DEFAULT_BALANCE_COST,
            transfer: DEFAULT_TRANSFER_COST,
            read_base_round_reward: DEFAULT_READ_BASE_ROUND_REWARD_COST,
        }
    }
}

impl ToBytes for MintCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, casper_types::bytesrepr::Error> {
        let mut ret = bytesrepr::unchecked_allocate_buffer(self);

        ret.append(&mut self.mint.to_bytes()?);
        ret.append(&mut self.reduce_total_supply.to_bytes()?);
        ret.append(&mut self.create.to_bytes()?);
        ret.append(&mut self.balance.to_bytes()?);
        ret.append(&mut self.transfer.to_bytes()?);
        ret.append(&mut self.read_base_round_reward.to_bytes()?);

        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.mint.serialized_length()
            + self.reduce_total_supply.serialized_length()
            + self.create.serialized_length()
            + self.balance.serialized_length()
            + self.transfer.serialized_length()
            + self.read_base_round_reward.serialized_length()
    }
}

impl FromBytes for MintCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
        let (mint, rem) = FromBytes::from_bytes(bytes)?;
        let (reduce_total_supply, rem) = FromBytes::from_bytes(rem)?;
        let (create, rem) = FromBytes::from_bytes(rem)?;
        let (balance, rem) = FromBytes::from_bytes(rem)?;
        let (transfer, rem) = FromBytes::from_bytes(rem)?;
        let (read_base_round_reward, rem) = FromBytes::from_bytes(rem)?;

        Ok((
            Self {
                mint,
                reduce_total_supply,
                create,
                balance,
                transfer,
                read_base_round_reward,
            },
            rem,
        ))
    }
}

impl Distribution<MintCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MintCosts {
        MintCosts {
            mint: rng.gen(),
            reduce_total_supply: rng.gen(),
            create: rng.gen(),
            balance: rng.gen(),
            transfer: rng.gen(),
            read_base_round_reward: rng.gen(),
        }
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::MintCosts;

    prop_compose! {
        pub fn mint_costs_arb()(
            mint in num::u32::ANY,
            reduce_total_supply in num::u32::ANY,
            create in num::u32::ANY,
            balance in num::u32::ANY,
            transfer in num::u32::ANY,
            read_base_round_reward in num::u32::ANY,
        ) -> MintCosts {
            MintCosts {
                mint,
                reduce_total_supply,
                create,
                balance,
                transfer,
                read_base_round_reward,
            }
        }
    }
}
