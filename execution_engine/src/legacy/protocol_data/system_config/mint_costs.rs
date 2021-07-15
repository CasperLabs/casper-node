use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::system_config::mint_costs::MintCosts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacyMintCosts {
    mint: u32,
    reduce_total_supply: u32,
    create: u32,
    balance: u32,
    transfer: u32,
    read_base_round_reward: u32,
}

impl From<LegacyMintCosts> for MintCosts {
    fn from(legacy_mint_costs: LegacyMintCosts) -> Self {
        MintCosts {
            mint: legacy_mint_costs.mint,
            reduce_total_supply: legacy_mint_costs.reduce_total_supply,
            create: legacy_mint_costs.create,
            balance: legacy_mint_costs.balance,
            transfer: legacy_mint_costs.transfer,
            read_base_round_reward: legacy_mint_costs.read_base_round_reward,
        }
    }
}

impl FromBytes for LegacyMintCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (mint, rem) = FromBytes::from_bytes(bytes)?;
        let (reduce_total_supply, rem) = FromBytes::from_bytes(rem)?;
        let (create, rem) = FromBytes::from_bytes(rem)?;
        let (balance, rem) = FromBytes::from_bytes(rem)?;
        let (transfer, rem) = FromBytes::from_bytes(rem)?;
        let (read_base_round_reward, rem) = FromBytes::from_bytes(rem)?;
        let legacy_mint_costs = LegacyMintCosts {
            mint,
            reduce_total_supply,
            create,
            balance,
            transfer,
            read_base_round_reward,
        };
        Ok((legacy_mint_costs, rem))
    }
}
