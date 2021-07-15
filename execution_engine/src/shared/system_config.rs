pub mod auction_costs;
pub mod handle_payment_costs;
pub mod mint_costs;
pub mod standard_payment_costs;

use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes};

use self::{
    auction_costs::AuctionCosts, handle_payment_costs::HandlePaymentCosts, mint_costs::MintCosts,
    standard_payment_costs::StandardPaymentCosts,
};
use crate::storage::protocol_data::DEFAULT_WASMLESS_TRANSFER_COST;

#[derive(FromPrimitive, ToPrimitive)]
enum SystemConfigKeys {
    WasmlessTransferCost = 100,
    AuctionCosts = 101,
    MintCosts = 102,
    HandlePaymentCosts = 103,
    StandardPaymentCosts = 104,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct SystemConfig {
    /// Wasmless transfer cost expressed in gas.
    wasmless_transfer_cost: u32,

    /// Configuration of auction entrypoint costs.
    auction_costs: AuctionCosts,

    /// Configuration of mint entrypoint costs.
    mint_costs: MintCosts,

    /// Configuration of handle payment entrypoint costs.
    handle_payment_costs: HandlePaymentCosts,

    /// Configuration of standard payment costs.
    standard_payment_costs: StandardPaymentCosts,
}

impl SystemConfig {
    pub fn new(
        wasmless_transfer_cost: u32,
        auction_costs: AuctionCosts,
        mint_costs: MintCosts,
        handle_payment_costs: HandlePaymentCosts,
        standard_payment_costs: StandardPaymentCosts,
    ) -> Self {
        Self {
            wasmless_transfer_cost,
            auction_costs,
            mint_costs,
            handle_payment_costs,
            standard_payment_costs,
        }
    }

    pub fn wasmless_transfer_cost(&self) -> u32 {
        self.wasmless_transfer_cost
    }

    pub fn auction_costs(&self) -> &AuctionCosts {
        &self.auction_costs
    }

    pub fn mint_costs(&self) -> &MintCosts {
        &self.mint_costs
    }

    pub fn handle_payment_costs(&self) -> &HandlePaymentCosts {
        &self.handle_payment_costs
    }

    pub fn standard_payment_costs(&self) -> &StandardPaymentCosts {
        &self.standard_payment_costs
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            wasmless_transfer_cost: DEFAULT_WASMLESS_TRANSFER_COST,
            auction_costs: AuctionCosts::default(),
            mint_costs: MintCosts::default(),
            handle_payment_costs: HandlePaymentCosts::default(),
            standard_payment_costs: StandardPaymentCosts::default(),
        }
    }
}

impl Distribution<SystemConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SystemConfig {
        SystemConfig {
            wasmless_transfer_cost: rng.gen(),
            auction_costs: rng.gen(),
            mint_costs: rng.gen(),
            handle_payment_costs: rng.gen(),
            standard_payment_costs: rng.gen(),
        }
    }
}

impl ToBytes for SystemConfig {
    fn to_bytes(&self) -> Result<Vec<u8>, casper_types::bytesrepr::Error> {
        let mut writer = StructWriter::new();

        writer.write_pair(
            SystemConfigKeys::WasmlessTransferCost,
            self.wasmless_transfer_cost,
        )?;
        writer.write_pair(SystemConfigKeys::AuctionCosts, self.auction_costs)?;
        writer.write_pair(SystemConfigKeys::MintCosts, self.mint_costs)?;
        writer.write_pair(
            SystemConfigKeys::HandlePaymentCosts,
            self.handle_payment_costs,
        )?;
        writer.write_pair(
            SystemConfigKeys::StandardPaymentCosts,
            self.standard_payment_costs,
        )?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[
            self.wasmless_transfer_cost.serialized_length(),
            self.auction_costs.serialized_length(),
            self.mint_costs.serialized_length(),
            self.handle_payment_costs.serialized_length(),
            self.standard_payment_costs.serialized_length(),
        ])
    }
}

impl FromBytes for SystemConfig {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), casper_types::bytesrepr::Error> {
        let mut system_config = SystemConfig::default();

        let mut reader = StructReader::new(bytes);

        while let Some(key) = reader.read_key()? {
            match SystemConfigKeys::from_u64(key) {
                Some(SystemConfigKeys::WasmlessTransferCost) => {
                    system_config.wasmless_transfer_cost = reader.read_value()?
                }
                Some(SystemConfigKeys::AuctionCosts) => {
                    system_config.auction_costs = reader.read_value()?
                }
                Some(SystemConfigKeys::MintCosts) => {
                    system_config.mint_costs = reader.read_value()?
                }
                Some(SystemConfigKeys::HandlePaymentCosts) => {
                    system_config.handle_payment_costs = reader.read_value()?
                }
                Some(SystemConfigKeys::StandardPaymentCosts) => {
                    system_config.standard_payment_costs = reader.read_value()?
                }
                None => reader.skip_value()?,
            }
        }
        Ok((system_config, reader.finish()))
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::{
        auction_costs::gens::auction_costs_arb,
        handle_payment_costs::gens::handle_payment_costs_arb, mint_costs::gens::mint_costs_arb,
        standard_payment_costs::gens::standard_payment_costs_arb, SystemConfig,
    };

    prop_compose! {
        pub fn system_config_arb()(
            wasmless_transfer_cost in num::u32::ANY,
            auction_costs in auction_costs_arb(),
            mint_costs in mint_costs_arb(),
            handle_payment_costs in handle_payment_costs_arb(),
            standard_payment_costs in standard_payment_costs_arb(),
        ) -> SystemConfig {
            SystemConfig {
                wasmless_transfer_cost,
                auction_costs,
                mint_costs,
                handle_payment_costs,
                standard_payment_costs,
            }
        }
    }
}
