use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::opcode_costs::OpcodeCosts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LegacyOpcodeCosts {
    bit: u32,
    add: u32,
    mul: u32,
    div: u32,
    load: u32,
    store: u32,
    op_const: u32,
    local: u32,
    global: u32,
    control_flow: u32,
    integer_comparison: u32,
    conversion: u32,
    unreachable: u32,
    nop: u32,
    current_memory: u32,
    grow_memory: u32,
    regular: u32,
}

impl From<LegacyOpcodeCosts> for OpcodeCosts {
    fn from(legacy_opcode_costs: LegacyOpcodeCosts) -> Self {
        OpcodeCosts {
            bit: legacy_opcode_costs.bit,
            add: legacy_opcode_costs.add,
            mul: legacy_opcode_costs.mul,
            div: legacy_opcode_costs.div,
            load: legacy_opcode_costs.load,
            store: legacy_opcode_costs.store,
            op_const: legacy_opcode_costs.op_const,
            local: legacy_opcode_costs.local,
            global: legacy_opcode_costs.global,
            control_flow: legacy_opcode_costs.control_flow,
            integer_comparison: legacy_opcode_costs.integer_comparison,
            conversion: legacy_opcode_costs.conversion,
            unreachable: legacy_opcode_costs.unreachable,
            nop: legacy_opcode_costs.nop,
            current_memory: legacy_opcode_costs.current_memory,
            grow_memory: legacy_opcode_costs.grow_memory,
            regular: legacy_opcode_costs.regular,
        }
    }
}

impl FromBytes for LegacyOpcodeCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (bit, bytes) = FromBytes::from_bytes(bytes)?;
        let (add, bytes) = FromBytes::from_bytes(bytes)?;
        let (mul, bytes) = FromBytes::from_bytes(bytes)?;
        let (div, bytes) = FromBytes::from_bytes(bytes)?;
        let (load, bytes) = FromBytes::from_bytes(bytes)?;
        let (store, bytes) = FromBytes::from_bytes(bytes)?;
        let (op_const, bytes) = FromBytes::from_bytes(bytes)?;
        let (local, bytes) = FromBytes::from_bytes(bytes)?;
        let (global, bytes) = FromBytes::from_bytes(bytes)?;
        let (control_flow, bytes) = FromBytes::from_bytes(bytes)?;
        let (integer_comparison, bytes) = FromBytes::from_bytes(bytes)?;
        let (conversion, bytes) = FromBytes::from_bytes(bytes)?;
        let (unreachable, bytes) = FromBytes::from_bytes(bytes)?;
        let (nop, bytes) = FromBytes::from_bytes(bytes)?;
        let (current_memory, bytes) = FromBytes::from_bytes(bytes)?;
        let (grow_memory, bytes) = FromBytes::from_bytes(bytes)?;
        let (regular, bytes) = FromBytes::from_bytes(bytes)?;
        let legacy_opcode_costs = LegacyOpcodeCosts {
            bit,
            add,
            mul,
            div,
            load,
            store,
            op_const,
            local,
            global,
            control_flow,
            integer_comparison,
            conversion,
            unreachable,
            nop,
            current_memory,
            grow_memory,
            regular,
        };
        Ok((legacy_opcode_costs, bytes))
    }
}
