use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::wasm_config::WasmConfig;

use super::{
    host_function_costs::LegacyHostFunctionCosts, opcode_costs::LegacyOpcodeCosts,
    storage_costs::LegacyStorageCosts,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacyWasmConfig {
    max_memory: u32,
    max_stack_height: u32,
    opcode_costs: LegacyOpcodeCosts,
    storage_costs: LegacyStorageCosts,
    host_function_costs: LegacyHostFunctionCosts,
}

impl From<LegacyWasmConfig> for WasmConfig {
    fn from(legacy_wasm_config: LegacyWasmConfig) -> Self {
        WasmConfig::new(
            legacy_wasm_config.max_memory,
            legacy_wasm_config.max_stack_height,
            legacy_wasm_config.opcode_costs.into(),
            legacy_wasm_config.storage_costs.into(),
            legacy_wasm_config.host_function_costs.into(),
        )
    }
}

impl FromBytes for LegacyWasmConfig {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (max_memory, rem) = FromBytes::from_bytes(bytes)?;
        let (max_stack_height, rem) = FromBytes::from_bytes(rem)?;
        let (opcode_costs, rem) = FromBytes::from_bytes(rem)?;
        let (storage_costs, rem) = FromBytes::from_bytes(rem)?;
        let (host_function_costs, rem) = FromBytes::from_bytes(rem)?;

        let legacy_wasm_config = LegacyWasmConfig {
            max_memory,
            max_stack_height,
            opcode_costs,
            storage_costs,
            host_function_costs,
        };

        Ok((legacy_wasm_config, rem))
    }
}
