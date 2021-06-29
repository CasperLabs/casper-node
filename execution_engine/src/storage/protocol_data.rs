use std::collections::BTreeMap;

use casper_types::{
    bytesrepr::{self, FromBytes, ToBytes},
    ContractHash, HashAddr,
};

use crate::shared::{core_config::CoreConfig, system_costs::SystemCosts, wasm_config::WasmConfig};

const DEFAULT_ADDRESS: [u8; 32] = [0; 32];
pub const DEFAULT_WASMLESS_TRANSFER_COST: u32 = 10_000;
pub const DEFAULT_MAX_ASSOCIATED_KEYS: u32 = 100;

/// Represents a protocol's data. Intended to be associated with a given protocol version.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ProtocolData {
    wasm_config: WasmConfig,
    core_config: CoreConfig,
    system_costs: SystemCosts,
    mint: ContractHash,
    handle_payment: ContractHash,
    standard_payment: ContractHash,
    auction: ContractHash,
}

/// Provides a default instance with non existing urefs and empty costs table.
///
/// Used in contexts where Handle Payment or Mint contract is not ready yet, and handle payment, and
/// mint installers are ran. For use with caution.
impl Default for ProtocolData {
    fn default() -> ProtocolData {
        ProtocolData {
            wasm_config: WasmConfig::default(),
            core_config: CoreConfig::default(),
            system_costs: SystemCosts::default(),
            mint: DEFAULT_ADDRESS.into(),
            handle_payment: DEFAULT_ADDRESS.into(),
            standard_payment: DEFAULT_ADDRESS.into(),
            auction: DEFAULT_ADDRESS.into(),
        }
    }
}

impl ProtocolData {
    /// Creates a new `ProtocolData` value from a given `WasmCosts` value.
    pub fn new(
        wasm_config: WasmConfig,
        core_config: CoreConfig,
        system_costs: SystemCosts,
        mint: ContractHash,
        handle_payment: ContractHash,
        standard_payment: ContractHash,
        auction: ContractHash,
    ) -> Self {
        ProtocolData {
            wasm_config,
            core_config,
            system_costs,
            mint,
            handle_payment,
            standard_payment,
            auction,
        }
    }

    /// Creates a new, partially-valid [`ProtocolData`] value where only the mint URef is known.
    ///
    /// Used during `commit_genesis` before all system contracts' URefs are known.
    pub fn partial_with_mint(mint: ContractHash) -> Self {
        ProtocolData {
            mint,
            ..Default::default()
        }
    }

    /// Creates a new, partially-valid [`ProtocolData`] value where all but the standard payment
    /// uref is known.
    ///
    /// Used during `commit_genesis` before all system contracts' URefs are known.
    pub fn partial_without_standard_payment(
        wasm_config: WasmConfig,
        mint: ContractHash,
        handle_payment: ContractHash,
    ) -> Self {
        ProtocolData {
            wasm_config,
            mint,
            handle_payment,
            ..Default::default()
        }
    }

    /// Gets the `WasmConfig` value from a given [`ProtocolData`] value.
    pub fn wasm_config(&self) -> &WasmConfig {
        &self.wasm_config
    }

    /// Gets the `CoreConfig` value from a given [`ProtocolData`] value.
    pub fn core_config(&self) -> &CoreConfig {
        &self.core_config
    }

    /// Gets the `SystemConfig` value from a given [`ProtocolData`] value.
    pub fn system_costs(&self) -> &SystemCosts {
        &self.system_costs
    }

    pub fn mint(&self) -> ContractHash {
        self.mint
    }

    pub fn handle_payment(&self) -> ContractHash {
        self.handle_payment
    }

    pub fn standard_payment(&self) -> ContractHash {
        self.standard_payment
    }

    pub fn auction(&self) -> ContractHash {
        self.auction
    }

    /// Retrieves all valid system contracts stored in protocol version
    pub fn system_contracts(&self) -> Vec<ContractHash> {
        let mut vec = Vec::with_capacity(4);
        if self.mint != DEFAULT_ADDRESS.into() {
            vec.push(self.mint)
        }
        if self.handle_payment != DEFAULT_ADDRESS.into() {
            vec.push(self.handle_payment)
        }
        if self.standard_payment != DEFAULT_ADDRESS.into() {
            vec.push(self.standard_payment)
        }
        if self.auction != DEFAULT_ADDRESS.into() {
            vec.push(self.auction)
        }
        vec
    }

    pub fn update_from(&mut self, updates: BTreeMap<ContractHash, ContractHash>) -> bool {
        for (old_hash, new_hash) in updates {
            if old_hash == self.mint {
                self.mint = new_hash;
            } else if old_hash == self.handle_payment {
                self.handle_payment = new_hash;
            } else if old_hash == self.standard_payment {
                self.standard_payment = new_hash;
            } else if old_hash == self.auction {
                self.auction = new_hash;
            } else {
                return false;
            }
        }
        true
    }
}

impl ToBytes for ProtocolData {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut ret = bytesrepr::unchecked_allocate_buffer(self);

        ret.append(&mut self.wasm_config.to_bytes()?);
        ret.append(&mut self.core_config.to_bytes()?);
        ret.append(&mut self.system_costs.to_bytes()?);
        ret.append(&mut self.mint.to_bytes()?);
        ret.append(&mut self.handle_payment.to_bytes()?);
        ret.append(&mut self.standard_payment.to_bytes()?);
        ret.append(&mut self.auction.to_bytes()?);

        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.wasm_config.serialized_length()
            + self.core_config.serialized_length()
            + self.system_costs.serialized_length()
            + self.mint.serialized_length()
            + self.handle_payment.serialized_length()
            + self.standard_payment.serialized_length()
            + self.auction.serialized_length()
    }
}

impl FromBytes for ProtocolData {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (wasm_config, rem) = WasmConfig::from_bytes(bytes)?;
        let (core_config, rem) = CoreConfig::from_bytes(rem)?;
        let (system_costs, rem) = FromBytes::from_bytes(rem)?;
        let (mint, rem) = HashAddr::from_bytes(rem)?;
        let (handle_payment, rem) = HashAddr::from_bytes(rem)?;
        let (standard_payment, rem) = HashAddr::from_bytes(rem)?;
        let (auction, rem) = HashAddr::from_bytes(rem)?;

        Ok((
            ProtocolData {
                wasm_config,
                core_config,
                system_costs,
                mint: mint.into(),
                handle_payment: handle_payment.into(),
                standard_payment: standard_payment.into(),
                auction: auction.into(),
            },
            rem,
        ))
    }
}

#[cfg(test)]
pub(crate) mod gens {
    use proptest::prop_compose;

    use crate::shared::{
        core_config::gens::core_config_arb, system_costs::gens::system_costs_arb,
        wasm_config::gens::wasm_config_arb,
    };
    use casper_types::gens;

    use super::ProtocolData;

    prop_compose! {
        pub fn protocol_data_arb()(
            wasm_config in wasm_config_arb(),
            core_config in core_config_arb(),
            system_costs in system_costs_arb(),
            mint in gens::u8_slice_32(),
            handle_payment in gens::u8_slice_32(),
            standard_payment in gens::u8_slice_32(),
            auction in gens::u8_slice_32(),
        ) -> ProtocolData {
            ProtocolData {
                wasm_config,
                core_config,
                system_costs,
                mint: mint.into(),
                handle_payment: handle_payment.into(),
                standard_payment: standard_payment.into(),
                auction: auction.into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::proptest;

    use crate::shared::{
        core_config::CoreConfig, system_costs::SystemCosts, wasm_config::WasmConfig,
    };
    use casper_types::{bytesrepr, ContractHash};

    use super::{gens, ProtocolData};

    #[test]
    fn should_return_all_system_contracts() {
        let mint_reference = [1u8; 32].into();
        let handle_payment_reference = [2u8; 32].into();
        let standard_payment_reference = [3u8; 32].into();
        let auction_reference = [4u8; 32].into();
        let protocol_data = {
            let wasm_config = WasmConfig::default();
            let core_config = CoreConfig::default();
            let system_costs = SystemCosts::default();
            ProtocolData::new(
                wasm_config,
                core_config,
                system_costs,
                mint_reference,
                handle_payment_reference,
                standard_payment_reference,
                auction_reference,
            )
        };

        let actual = {
            let mut items = protocol_data.system_contracts();
            items.sort_unstable();
            items
        };

        assert_eq!(actual.len(), 4);
        assert_eq!(actual[0], mint_reference);
        assert_eq!(actual[1], handle_payment_reference);
        assert_eq!(actual[2], standard_payment_reference);
        assert_eq!(actual[3], auction_reference);
    }

    #[test]
    fn should_return_only_valid_system_contracts() {
        let expected: Vec<ContractHash> = vec![];
        assert_eq!(ProtocolData::default().system_contracts(), expected);

        let mint_reference = [0u8; 32].into(); // <-- invalid addr
        let handle_payment_reference = [2u8; 32].into();
        let standard_payment_reference = [3u8; 32].into();
        let auction_reference = [4u8; 32].into();
        let protocol_data = {
            let wasm_config = WasmConfig::default();
            let core_config = CoreConfig::default();
            let system_costs = SystemCosts::default();
            ProtocolData::new(
                wasm_config,
                core_config,
                system_costs,
                mint_reference,
                handle_payment_reference,
                standard_payment_reference,
                auction_reference,
            )
        };

        let actual = {
            let mut items = protocol_data.system_contracts();
            items.sort_unstable();
            items
        };

        assert_eq!(actual.len(), 3);
        assert_eq!(actual[0], handle_payment_reference);
        assert_eq!(actual[1], standard_payment_reference);
        assert_eq!(actual[2], auction_reference);
    }

    proptest! {
        #[test]
        fn should_serialize_and_deserialize_with_arbitrary_values(
            protocol_data in gens::protocol_data_arb()
        ) {
            bytesrepr::test_serialization_roundtrip(&protocol_data);
        }
    }
}
