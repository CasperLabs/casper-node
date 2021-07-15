use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes};

pub const DEFAULT_MINT_COST: u32 = 10_000;
pub const DEFAULT_REDUCE_TOTAL_SUPPLY_COST: u32 = 10_000;
pub const DEFAULT_CREATE_COST: u32 = 10_000;
pub const DEFAULT_BALANCE_COST: u32 = 10_000;
pub const DEFAULT_TRANSFER_COST: u32 = 10_000;
pub const DEFAULT_READ_BASE_ROUND_REWARD_COST: u32 = 10_000;

#[derive(FromPrimitive, ToPrimitive)]
enum MintCostsKeys {
    Mint = 100,
    ReduceTotalSupply = 101,
    Create = 102,
    Balance = 103,
    Transfer = 104,
    ReadBaseRoundReward = 105,
}

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
        let mut writer = StructWriter::new();

        writer.write_pair(MintCostsKeys::Mint, self.mint)?;
        writer.write_pair(MintCostsKeys::ReduceTotalSupply, self.reduce_total_supply)?;
        writer.write_pair(MintCostsKeys::Create, self.create)?;
        writer.write_pair(MintCostsKeys::Balance, self.balance)?;
        writer.write_pair(MintCostsKeys::Transfer, self.transfer)?;
        writer.write_pair(
            MintCostsKeys::ReadBaseRoundReward,
            self.read_base_round_reward,
        )?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[
            self.mint.serialized_length(),
            self.reduce_total_supply.serialized_length(),
            self.create.serialized_length(),
            self.balance.serialized_length(),
            self.transfer.serialized_length(),
            self.read_base_round_reward.serialized_length(),
        ])
    }
}

impl FromBytes for MintCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
        let mut mint_costs = MintCosts::default();

        let mut reader = StructReader::new(bytes);

        while let Some(key) = reader.read_key()? {
            match MintCostsKeys::from_u64(key) {
                Some(MintCostsKeys::Mint) => mint_costs.mint = reader.read_value()?,
                Some(MintCostsKeys::ReduceTotalSupply) => {
                    mint_costs.reduce_total_supply = reader.read_value()?
                }
                Some(MintCostsKeys::Create) => mint_costs.create = reader.read_value()?,
                Some(MintCostsKeys::Balance) => mint_costs.balance = reader.read_value()?,
                Some(MintCostsKeys::Transfer) => mint_costs.transfer = reader.read_value()?,
                Some(MintCostsKeys::ReadBaseRoundReward) => {
                    mint_costs.read_base_round_reward = reader.read_value()?
                }
                None => reader.skip_value()?,
            }
        }

        Ok((mint_costs, reader.finish()))
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
