//! Routine for decoding a legacy [`ProtocolData`] serialization format.
mod host_function_costs;
mod opcode_costs;
mod storage_costs;
mod system_config;
mod wasm_config;

use casper_types::{
    bytesrepr::{self, FromBytes},
    ContractHash, ProtocolVersion,
};

use crate::storage::protocol_data::ProtocolData;

use self::{system_config::LegacySystemConfig, wasm_config::LegacyWasmConfig};

use super::types::LegacyHashAddr;

/// Protocol versions lower than or equal to this value should be deserialized with a legacy format.
pub const LEGACY_PROTOCOL_DATA_VERSION: ProtocolVersion = ProtocolVersion::from_parts(1, 2, 0);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct LegacyProtocolData {
    wasm_config: LegacyWasmConfig,
    system_config: LegacySystemConfig,
    mint: ContractHash,
    handle_payment: ContractHash,
    standard_payment: ContractHash,
    auction: ContractHash,
}

impl From<LegacyProtocolData> for ProtocolData {
    fn from(legacy_protocol_data: LegacyProtocolData) -> Self {
        ProtocolData::new(
            legacy_protocol_data.wasm_config.into(),
            legacy_protocol_data.system_config.into(),
            legacy_protocol_data.mint,
            legacy_protocol_data.handle_payment,
            legacy_protocol_data.standard_payment,
            legacy_protocol_data.auction,
        )
    }
}

impl FromBytes for LegacyProtocolData {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (wasm_config, rem) = FromBytes::from_bytes(bytes)?;
        let (system_config, rem) = FromBytes::from_bytes(rem)?;
        let (mint, rem) = LegacyHashAddr::from_bytes(rem)?;
        let (handle_payment, rem) = LegacyHashAddr::from_bytes(rem)?;
        let (standard_payment, rem) = LegacyHashAddr::from_bytes(rem)?;
        let (auction, rem) = LegacyHashAddr::from_bytes(rem)?;

        let legacy_protocol_data = LegacyProtocolData {
            wasm_config,
            system_config,
            mint: mint.into(),
            handle_payment: handle_payment.into(),
            standard_payment: standard_payment.into(),
            auction: auction.into(),
        };

        Ok((legacy_protocol_data, rem))
    }
}
