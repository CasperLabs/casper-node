use std::collections::BTreeMap;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;

use casper_types::{
    bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes},
    ContractHash, ProtocolVersion,
};

use crate::{
    legacy::protocol_data::{LegacyProtocolData, LEGACY_PROTOCOL_DATA_VERSION},
    shared::{system_config::SystemConfig, wasm_config::WasmConfig},
};

const DEFAULT_ADDRESS: [u8; 32] = [0; 32];
pub const DEFAULT_WASMLESS_TRANSFER_COST: u32 = 10_000;

/// Represents a protocol's data. Intended to be associated with a given protocol version.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ProtocolData {
    wasm_config: WasmConfig,
    system_config: SystemConfig,
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
            system_config: SystemConfig::default(),
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
        system_costs: SystemConfig,
        mint: ContractHash,
        handle_payment: ContractHash,
        standard_payment: ContractHash,
        auction: ContractHash,
    ) -> Self {
        ProtocolData {
            wasm_config,
            system_config: system_costs,
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

    /// Gets the `SystemConfig` value from a given [`ProtocolData`] value.
    pub fn system_config(&self) -> &SystemConfig {
        &self.system_config
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

#[derive(FromPrimitive, ToPrimitive)]
enum ProtocolDataKeys {
    WasmConfig = 100,
    SystemConfig = 101,
    Mint = 102,
    HandlePayment = 103,
    StandardPayment = 104,
    Auction = 105,
}

impl ToBytes for ProtocolData {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut writer = StructWriter::new();

        writer.write_pair(ProtocolDataKeys::WasmConfig, self.wasm_config)?;
        writer.write_pair(ProtocolDataKeys::SystemConfig, self.system_config)?;
        writer.write_pair(ProtocolDataKeys::Mint, self.mint)?;
        writer.write_pair(ProtocolDataKeys::HandlePayment, self.handle_payment)?;
        writer.write_pair(ProtocolDataKeys::StandardPayment, self.standard_payment)?;
        writer.write_pair(ProtocolDataKeys::Auction, self.auction)?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[
            self.wasm_config.serialized_length(),
            self.system_config.serialized_length(),
            self.mint.serialized_length(),
            self.handle_payment.serialized_length(),
            self.standard_payment.serialized_length(),
            self.auction.serialized_length(),
        ])
    }
}

impl FromBytes for ProtocolData {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let mut reader = StructReader::new(bytes);

        let mut protocol_data = ProtocolData::default();

        while let Some(key) = reader.read_key()? {
            match ProtocolDataKeys::from_u64(key) {
                Some(ProtocolDataKeys::WasmConfig) => {
                    protocol_data.wasm_config = reader.read_value()?
                }
                Some(ProtocolDataKeys::SystemConfig) => {
                    protocol_data.system_config = reader.read_value()?
                }
                Some(ProtocolDataKeys::Mint) => protocol_data.mint = reader.read_value()?,
                Some(ProtocolDataKeys::HandlePayment) => {
                    protocol_data.handle_payment = reader.read_value()?
                }
                Some(ProtocolDataKeys::StandardPayment) => {
                    protocol_data.standard_payment = reader.read_value()?
                }
                Some(ProtocolDataKeys::Auction) => protocol_data.auction = reader.read_value()?,
                None => reader.skip_value()?,
            }
        }

        Ok((protocol_data, reader.finish()))
    }
}

/// Deserializes [`ProtocolData`] with respect to different data formats present across protocol
/// versions.
pub(crate) fn get_versioned_protocol_data(
    protocol_version: &ProtocolVersion,
    bytes: Vec<u8>,
) -> Result<ProtocolData, bytesrepr::Error> {
    if protocol_version <= &LEGACY_PROTOCOL_DATA_VERSION {
        // Old, legacy data format
        match bytesrepr::deserialize::<LegacyProtocolData>(bytes.clone()) {
            Ok(legacy_protocol_data) => {
                // Legacy protocol data deserialized with a success (i.e. existing network)
                Ok(ProtocolData::from(legacy_protocol_data))
            }
            Err(bytesrepr::Error::LeftOverBytes) => {
                // Any LeftOverBytes error during deserialization of legacy protocol data indicates
                // that the newer format is longer than the legacy one.
                // This can happen when (for example) new network starts with a genesis using a
                // lower than or equal to LEGACY_PROTOCOL_DATA_VERSION.
                //
                // Genesis process intentionally serializes new format
                // instead of legacy, where deserialization considers old format based on version
                // just for migration purposes.
                //
                // This choice is made intentionally to enforce new serialization format by default.
                // The benefit of this approach is that a) we don't need to maintain
                // legacy serialization code and also b) special conversions between
                // LegacyProtocolData and ProtocolData should it change over time and c) we should
                // be able to remove legacy code on next major release
                bytesrepr::deserialize(bytes)
            }
            Err(error) => Err(error),
        }
    } else {
        // Future-proof data format
        bytesrepr::deserialize(bytes)
    }
}

#[cfg(test)]
pub(crate) mod gens {
    use proptest::prop_compose;

    use crate::shared::{
        system_config::gens::system_config_arb, wasm_config::gens::wasm_config_arb,
    };
    use casper_types::gens;

    use super::ProtocolData;

    prop_compose! {
        pub fn protocol_data_arb()(
            wasm_config in wasm_config_arb(),
            system_config in system_config_arb(),
            mint in gens::u8_slice_32(),
            handle_payment in gens::u8_slice_32(),
            standard_payment in gens::u8_slice_32(),
            auction in gens::u8_slice_32(),
        ) -> ProtocolData {
            ProtocolData {
                wasm_config,
                system_config,
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

    use crate::shared::{system_config::SystemConfig, wasm_config::WasmConfig};
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
            let system_config = SystemConfig::default();
            ProtocolData::new(
                wasm_config,
                system_config,
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
            let system_config = SystemConfig::default();
            ProtocolData::new(
                wasm_config,
                system_config,
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
