use std::collections::BTreeMap;

use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use pwasm_utils::rules::{InstructionType, Metering, Set};
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{
    self, FromBytes, StructReader, StructWriter, ToBytes, U32_SERIALIZED_LENGTH,
};

pub const DEFAULT_BIT_COST: u32 = 300;
pub const DEFAULT_ADD_COST: u32 = 210;
pub const DEFAULT_MUL_COST: u32 = 240;
pub const DEFAULT_DIV_COST: u32 = 320;
pub const DEFAULT_LOAD_COST: u32 = 2_500;
pub const DEFAULT_STORE_COST: u32 = 4_700;
pub const DEFAULT_CONST_COST: u32 = 110;
pub const DEFAULT_LOCAL_COST: u32 = 390;
pub const DEFAULT_GLOBAL_COST: u32 = 390;
pub const DEFAULT_CONTROL_FLOW_COST: u32 = 440;
pub const DEFAULT_INTEGER_COMPARISON_COST: u32 = 250;
pub const DEFAULT_CONVERSION_COST: u32 = 420;
pub const DEFAULT_UNREACHABLE_COST: u32 = 270;
pub const DEFAULT_NOP_COST: u32 = 200; // TODO: This value is not researched
pub const DEFAULT_CURRENT_MEMORY_COST: u32 = 290;
pub const DEFAULT_GROW_MEMORY_COST: u32 = 240_000;
pub const DEFAULT_REGULAR_COST: u32 = 210;

const NUM_FIELDS: usize = 17;
pub const OPCODE_COSTS_SERIALIZED_LENGTH: usize = NUM_FIELDS * U32_SERIALIZED_LENGTH;

#[derive(ToPrimitive, FromPrimitive)]
enum OpcodeCostsKey {
    Bit = 100,
    Add = 101,
    Mul = 102,
    Div = 103,
    Load = 104,
    Store = 105,
    Const = 106,
    Local = 107,
    Global = 108,
    ControlFlow = 109,
    IntegerComparison = 110,
    Conversion = 111,
    Unreachable = 112,
    Nop = 113,
    CurrentMemory = 114,
    GrowMemory = 115,
    Regular = 116,
}

// Taken (partially) from parity-ethereum
#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct OpcodeCosts {
    /// Bit operations multiplier.
    pub bit: u32,
    /// Arithmetic add operations multiplier.
    pub add: u32,
    /// Mul operations multiplier.
    pub mul: u32,
    /// Div operations multiplier.
    pub div: u32,
    /// Memory load operation multiplier.
    pub load: u32,
    /// Memory store operation multiplier.
    pub store: u32,
    /// Const operation multiplier.
    #[serde(rename = "const")]
    pub op_const: u32,
    /// Local operations multiplier.
    pub local: u32,
    /// Global operations multiplier.
    pub global: u32,
    /// Control flow operations multiplier.
    pub control_flow: u32,
    /// Integer operations multiplier.
    pub integer_comparison: u32,
    /// Conversion operations multiplier.
    pub conversion: u32,
    /// Unreachable operation multiplier.
    pub unreachable: u32,
    /// Nop operation multiplier.
    pub nop: u32,
    /// Get current memory operation multiplier.
    pub current_memory: u32,
    /// Grow memory cost, per page (64kb)
    pub grow_memory: u32,
    /// Regular opcode cost
    pub regular: u32,
}

impl OpcodeCosts {
    pub(crate) fn to_set(self) -> Set {
        let meterings = {
            let mut tmp = BTreeMap::new();
            tmp.insert(InstructionType::Bit, Metering::Fixed(self.bit));
            tmp.insert(InstructionType::Add, Metering::Fixed(self.add));
            tmp.insert(InstructionType::Mul, Metering::Fixed(self.mul));
            tmp.insert(InstructionType::Div, Metering::Fixed(self.div));
            tmp.insert(InstructionType::Load, Metering::Fixed(self.load));
            tmp.insert(InstructionType::Store, Metering::Fixed(self.store));
            tmp.insert(InstructionType::Const, Metering::Fixed(self.op_const));
            tmp.insert(InstructionType::Local, Metering::Fixed(self.local));
            tmp.insert(InstructionType::Global, Metering::Fixed(self.global));
            tmp.insert(
                InstructionType::ControlFlow,
                Metering::Fixed(self.control_flow),
            );
            tmp.insert(
                InstructionType::IntegerComparison,
                Metering::Fixed(self.integer_comparison),
            );
            tmp.insert(
                InstructionType::Conversion,
                Metering::Fixed(self.conversion),
            );
            tmp.insert(
                InstructionType::Unreachable,
                Metering::Fixed(self.unreachable),
            );
            tmp.insert(InstructionType::Nop, Metering::Fixed(self.nop));
            tmp.insert(
                InstructionType::CurrentMemory,
                Metering::Fixed(self.current_memory),
            );
            tmp.insert(
                InstructionType::GrowMemory,
                Metering::Fixed(self.grow_memory),
            );

            // Instructions Float, FloatComparison, FloatConst, FloatConversion are omitted here
            // because we're using `with_forbidden_floats` below.

            tmp
        };
        Set::new(self.regular, meterings)
            .with_grow_cost(self.grow_memory)
            .with_forbidden_floats()
    }
}

impl Default for OpcodeCosts {
    fn default() -> Self {
        OpcodeCosts {
            bit: DEFAULT_BIT_COST,
            add: DEFAULT_ADD_COST,
            mul: DEFAULT_MUL_COST,
            div: DEFAULT_DIV_COST,
            load: DEFAULT_LOAD_COST,
            store: DEFAULT_STORE_COST,
            op_const: DEFAULT_CONST_COST,
            local: DEFAULT_LOCAL_COST,
            global: DEFAULT_GLOBAL_COST,
            control_flow: DEFAULT_CONTROL_FLOW_COST,
            integer_comparison: DEFAULT_INTEGER_COMPARISON_COST,
            conversion: DEFAULT_CONVERSION_COST,
            unreachable: DEFAULT_UNREACHABLE_COST,
            nop: DEFAULT_NOP_COST,
            current_memory: DEFAULT_CURRENT_MEMORY_COST,
            grow_memory: DEFAULT_GROW_MEMORY_COST,
            regular: DEFAULT_REGULAR_COST,
        }
    }
}

impl Distribution<OpcodeCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> OpcodeCosts {
        OpcodeCosts {
            bit: rng.gen(),
            add: rng.gen(),
            mul: rng.gen(),
            div: rng.gen(),
            load: rng.gen(),
            store: rng.gen(),
            op_const: rng.gen(),
            local: rng.gen(),
            global: rng.gen(),
            control_flow: rng.gen(),
            integer_comparison: rng.gen(),
            conversion: rng.gen(),
            unreachable: rng.gen(),
            nop: rng.gen(),
            current_memory: rng.gen(),
            grow_memory: rng.gen(),
            regular: rng.gen(),
        }
    }
}

impl ToBytes for OpcodeCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut writer = StructWriter::new();

        writer.write_pair(OpcodeCostsKey::Bit, self.bit)?;
        writer.write_pair(OpcodeCostsKey::Add, self.add)?;
        writer.write_pair(OpcodeCostsKey::Mul, self.mul)?;
        writer.write_pair(OpcodeCostsKey::Div, self.div)?;
        writer.write_pair(OpcodeCostsKey::Load, self.load)?;
        writer.write_pair(OpcodeCostsKey::Store, self.store)?;
        writer.write_pair(OpcodeCostsKey::Const, self.op_const)?;
        writer.write_pair(OpcodeCostsKey::Local, self.local)?;
        writer.write_pair(OpcodeCostsKey::Global, self.global)?;
        writer.write_pair(OpcodeCostsKey::ControlFlow, self.control_flow)?;
        writer.write_pair(OpcodeCostsKey::IntegerComparison, self.integer_comparison)?;
        writer.write_pair(OpcodeCostsKey::Conversion, self.conversion)?;
        writer.write_pair(OpcodeCostsKey::Unreachable, self.unreachable)?;
        writer.write_pair(OpcodeCostsKey::Nop, self.nop)?;
        writer.write_pair(OpcodeCostsKey::CurrentMemory, self.current_memory)?;
        writer.write_pair(OpcodeCostsKey::GrowMemory, self.grow_memory)?;
        writer.write_pair(OpcodeCostsKey::Regular, self.regular)?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[
            self.bit.serialized_length(),
            self.add.serialized_length(),
            self.mul.serialized_length(),
            self.div.serialized_length(),
            self.load.serialized_length(),
            self.store.serialized_length(),
            self.op_const.serialized_length(),
            self.local.serialized_length(),
            self.global.serialized_length(),
            self.control_flow.serialized_length(),
            self.integer_comparison.serialized_length(),
            self.conversion.serialized_length(),
            self.unreachable.serialized_length(),
            self.nop.serialized_length(),
            self.current_memory.serialized_length(),
            self.grow_memory.serialized_length(),
            self.regular.serialized_length(),
        ])
    }
}

impl FromBytes for OpcodeCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let mut opcode_costs = OpcodeCosts::default();

        let mut reader = StructReader::new(bytes);

        while let Some(key) = reader.read_key()? {
            match OpcodeCostsKey::from_u64(key) {
                Some(OpcodeCostsKey::Bit) => opcode_costs.bit = reader.read_value()?,
                Some(OpcodeCostsKey::Add) => opcode_costs.add = reader.read_value()?,
                Some(OpcodeCostsKey::Mul) => opcode_costs.mul = reader.read_value()?,
                Some(OpcodeCostsKey::Div) => opcode_costs.div = reader.read_value()?,
                Some(OpcodeCostsKey::Load) => opcode_costs.load = reader.read_value()?,
                Some(OpcodeCostsKey::Store) => opcode_costs.store = reader.read_value()?,
                Some(OpcodeCostsKey::Const) => opcode_costs.op_const = reader.read_value()?,
                Some(OpcodeCostsKey::Local) => opcode_costs.local = reader.read_value()?,
                Some(OpcodeCostsKey::Global) => opcode_costs.global = reader.read_value()?,
                Some(OpcodeCostsKey::ControlFlow) => {
                    opcode_costs.control_flow = reader.read_value()?
                }
                Some(OpcodeCostsKey::IntegerComparison) => {
                    opcode_costs.integer_comparison = reader.read_value()?
                }
                Some(OpcodeCostsKey::Conversion) => {
                    opcode_costs.conversion = reader.read_value()?
                }
                Some(OpcodeCostsKey::Unreachable) => {
                    opcode_costs.unreachable = reader.read_value()?
                }
                Some(OpcodeCostsKey::Nop) => opcode_costs.nop = reader.read_value()?,
                Some(OpcodeCostsKey::CurrentMemory) => {
                    opcode_costs.current_memory = reader.read_value()?
                }
                Some(OpcodeCostsKey::GrowMemory) => {
                    opcode_costs.grow_memory = reader.read_value()?
                }
                Some(OpcodeCostsKey::Regular) => opcode_costs.regular = reader.read_value()?,
                None => reader.skip_value()?,
            }
        }

        Ok((opcode_costs, reader.finish()))
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use crate::shared::opcode_costs::OpcodeCosts;

    prop_compose! {
        pub fn opcode_costs_arb()(
            bit in num::u32::ANY,
            add in num::u32::ANY,
            mul in num::u32::ANY,
            div in num::u32::ANY,
            load in num::u32::ANY,
            store in num::u32::ANY,
            op_const in num::u32::ANY,
            local in num::u32::ANY,
            global in num::u32::ANY,
            control_flow in num::u32::ANY,
            integer_comparison in num::u32::ANY,
            conversion in num::u32::ANY,
            unreachable in num::u32::ANY,
            nop in num::u32::ANY,
            current_memory in num::u32::ANY,
            grow_memory in num::u32::ANY,
            regular in num::u32::ANY,
        ) -> OpcodeCosts {
            OpcodeCosts {
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::proptest;

    use casper_types::bytesrepr;

    use super::gens;

    proptest! {
        #[test]
        fn should_serialize_and_deserialize_with_arbitrary_values(
            opcode_costs in gens::opcode_costs_arb()
        ) {
            bytesrepr::test_serialization_roundtrip(&opcode_costs);
        }
    }
}
