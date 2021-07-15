//! This file provides types to allow conversion from an EE `ExecutionResult` into a similar type
//! which can be serialized to a valid binary or JSON representation.
//!
//! It is stored as metadata related to a given deploy, and made available to clients via the
//! JSON-RPC API.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(feature = "std")]
use once_cell::sync::Lazy;
use rand::{
    distributions::{Distribution, Standard},
    seq::SliceRandom,
    Rng,
};
#[cfg(feature = "std")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use crate::KEY_HASH_LENGTH;
use crate::{
    account::AccountHash,
    bytesrepr::{self, FromBytes, ToBytes, U8_SERIALIZED_LENGTH},
    system::auction::{Bid, EraInfo, UnbondingPurse},
    CLValue, DeployInfo, NamedKey, Transfer, TransferAddr, U128, U256, U512,
};

/// Constants to track ExecutionResult serialization.
const EXECUTION_RESULT_FAILURE_TAG: u8 = 0;
const EXECUTION_RESULT_SUCCESS_TAG: u8 = 1;

/// Constants to track operation serialization.
const OP_READ_TAG: u8 = 0;
const OP_WRITE_TAG: u8 = 1;
const OP_ADD_TAG: u8 = 2;
const OP_NOOP_TAG: u8 = 3;

/// Constants to track Transform serialization.
const TRANSFORM_IDENTITY_TAG: u8 = 0;
const TRANSFORM_WRITE_CLVALUE_TAG: u8 = 1;
const TRANSFORM_WRITE_ACCOUNT_TAG: u8 = 2;
const TRANSFORM_WRITE_CONTRACT_WASM_TAG: u8 = 3;
const TRANSFORM_WRITE_CONTRACT_TAG: u8 = 4;
const TRANSFORM_WRITE_CONTRACT_PACKAGE_TAG: u8 = 5;
const TRANSFORM_WRITE_DEPLOY_INFO_TAG: u8 = 6;
const TRANSFORM_WRITE_TRANSFER_TAG: u8 = 7;
const TRANSFORM_WRITE_ERA_INFO_TAG: u8 = 8;
const TRANSFORM_WRITE_BID_TAG: u8 = 9;
const TRANSFORM_WRITE_WITHDRAW_TAG: u8 = 10;
const TRANSFORM_ADD_INT32_TAG: u8 = 11;
const TRANSFORM_ADD_UINT64_TAG: u8 = 12;
const TRANSFORM_ADD_UINT128_TAG: u8 = 13;
const TRANSFORM_ADD_UINT256_TAG: u8 = 14;
const TRANSFORM_ADD_UINT512_TAG: u8 = 15;
const TRANSFORM_ADD_KEYS_TAG: u8 = 16;
const TRANSFORM_FAILURE_TAG: u8 = 17;

#[cfg(feature = "std")]
static EXECUTION_RESULT: Lazy<ExecutionResult> = Lazy::new(|| {
    let operations = vec![
        Operation {
            key: "account-hash-2c4a11c062a8a337bfc97e27fd66291caeb2c65865dcb5d3ef3759c4c97efecb"
                .to_string(),
            kind: OpKind::Write,
        },
        Operation {
            key: "deploy-af684263911154d26fa05be9963171802801a0b6aff8f199b7391eacb8edc9e1"
                .to_string(),
            kind: OpKind::Read,
        },
    ];

    let transforms = vec![
        TransformEntry {
            key: "uref-2c4a11c062a8a337bfc97e27fd66291caeb2c65865dcb5d3ef3759c4c97efecb-007"
                .to_string(),
            transform: Transform::AddUInt64(8u64),
        },
        TransformEntry {
            key: "deploy-af684263911154d26fa05be9963171802801a0b6aff8f199b7391eacb8edc9e1"
                .to_string(),
            transform: Transform::Identity,
        },
    ];

    let effect = ExecutionEffect {
        operations,
        transforms,
    };

    let transfers = vec![
        TransferAddr::new([89; KEY_HASH_LENGTH]),
        TransferAddr::new([130; KEY_HASH_LENGTH]),
    ];

    ExecutionResult::Success {
        effect,
        transfers,
        cost: U512::from(123_456),
    }
});

/// The result of executing a single deploy.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub enum ExecutionResult {
    /// The result of a failed execution.
    Failure {
        /// The effect of executing the deploy.
        effect: ExecutionEffect,
        /// A record of Transfers performed while executing the deploy.
        transfers: Vec<TransferAddr>,
        /// The cost of executing the deploy.
        cost: U512,
        /// The error message associated with executing the deploy.
        error_message: String,
    },
    /// The result of a successful execution.
    Success {
        /// The effect of executing the deploy.
        effect: ExecutionEffect,
        /// A record of Transfers performed while executing the deploy.
        transfers: Vec<TransferAddr>,
        /// The cost of executing the deploy.
        cost: U512,
    },
}

impl ExecutionResult {
    // This method is not intended to be used by third party crates.
    #[doc(hidden)]
    #[cfg(feature = "std")]
    pub fn example() -> &'static Self {
        &*EXECUTION_RESULT
    }
}

impl Distribution<ExecutionResult> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ExecutionResult {
        let op_count = rng.gen_range(0..6);
        let mut operations = Vec::new();
        for _ in 0..op_count {
            let op = [OpKind::Read, OpKind::Add, OpKind::NoOp, OpKind::Write]
                .choose(rng)
                .unwrap();
            operations.push(Operation {
                key: rng.gen::<u64>().to_string(),
                kind: *op,
            });
        }

        let transform_count = rng.gen_range(0..6);
        let mut transforms = Vec::new();
        for _ in 0..transform_count {
            transforms.push(TransformEntry {
                key: rng.gen::<u64>().to_string(),
                transform: rng.gen(),
            });
        }

        let effect = ExecutionEffect {
            operations,
            transforms,
        };

        let transfer_count = rng.gen_range(0..6);
        let mut transfers = vec![];
        for _ in 0..transfer_count {
            transfers.push(TransferAddr::new(rng.gen()))
        }

        if rng.gen() {
            ExecutionResult::Failure {
                effect,
                transfers,
                cost: rng.gen::<u64>().into(),
                error_message: format!("Error message {}", rng.gen::<u64>()),
            }
        } else {
            ExecutionResult::Success {
                effect,
                transfers,
                cost: rng.gen::<u64>().into(),
            }
        }
    }
}

impl ToBytes for ExecutionResult {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        match self {
            ExecutionResult::Failure {
                effect,
                transfers,
                cost,
                error_message,
            } => {
                buffer.push(EXECUTION_RESULT_FAILURE_TAG);
                buffer.extend(effect.to_bytes()?);
                buffer.extend(transfers.to_bytes()?);
                buffer.extend(cost.to_bytes()?);
                buffer.extend(error_message.to_bytes()?);
            }
            ExecutionResult::Success {
                effect,
                transfers,
                cost,
            } => {
                buffer.push(EXECUTION_RESULT_SUCCESS_TAG);
                buffer.extend(effect.to_bytes()?);
                buffer.extend(transfers.to_bytes()?);
                buffer.extend(cost.to_bytes()?);
            }
        }
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
            + match self {
                ExecutionResult::Failure {
                    effect,
                    transfers,
                    cost,
                    error_message,
                } => {
                    effect.serialized_length()
                        + transfers.serialized_length()
                        + cost.serialized_length()
                        + error_message.serialized_length()
                }
                ExecutionResult::Success {
                    effect,
                    transfers,
                    cost,
                } => {
                    effect.serialized_length()
                        + transfers.serialized_length()
                        + cost.serialized_length()
                }
            }
    }
}

impl FromBytes for ExecutionResult {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = u8::from_bytes(bytes)?;
        match tag {
            EXECUTION_RESULT_FAILURE_TAG => {
                let (effect, remainder) = ExecutionEffect::from_bytes(remainder)?;
                let (transfers, remainder) = Vec::<TransferAddr>::from_bytes(remainder)?;
                let (cost, remainder) = U512::from_bytes(remainder)?;
                let (error_message, remainder) = String::from_bytes(remainder)?;
                let execution_result = ExecutionResult::Failure {
                    effect,
                    transfers,
                    cost,
                    error_message,
                };
                Ok((execution_result, remainder))
            }
            EXECUTION_RESULT_SUCCESS_TAG => {
                let (effect, remainder) = ExecutionEffect::from_bytes(remainder)?;
                let (transfers, remainder) = Vec::<TransferAddr>::from_bytes(remainder)?;
                let (cost, remainder) = U512::from_bytes(remainder)?;
                let execution_result = ExecutionResult::Success {
                    effect,
                    transfers,
                    cost,
                };
                Ok((execution_result, remainder))
            }
            _ => Err(bytesrepr::Error::Formatting),
        }
    }
}

/// The effect of executing a single deploy.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Default, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExecutionEffect {
    /// The resulting operations.
    pub operations: Vec<Operation>,
    /// The resulting transformations.
    pub transforms: Vec<TransformEntry>,
}

impl ToBytes for ExecutionEffect {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.operations.to_bytes()?);
        buffer.extend(self.transforms.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.operations.serialized_length() + self.transforms.serialized_length()
    }
}

impl FromBytes for ExecutionEffect {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (operations, remainder) = Vec::<Operation>::from_bytes(bytes)?;
        let (transforms, remainder) = Vec::<TransformEntry>::from_bytes(remainder)?;
        let execution_effect = ExecutionEffect {
            operations,
            transforms,
        };
        Ok((execution_effect, remainder))
    }
}

/// An operation performed while executing a deploy.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Operation {
    /// The formatted string of the `Key`.
    pub key: String,
    /// The type of operation.
    pub kind: OpKind,
}

impl ToBytes for Operation {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.key.to_bytes()?);
        buffer.extend(self.kind.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.key.serialized_length() + self.kind.serialized_length()
    }
}

impl FromBytes for Operation {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (key, remainder) = String::from_bytes(bytes)?;
        let (kind, remainder) = OpKind::from_bytes(remainder)?;
        let operation = Operation { key, kind };
        Ok((operation, remainder))
    }
}

/// The type of operation performed while executing a deploy.
#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub enum OpKind {
    /// A read operation.
    Read,
    /// A write operation.
    Write,
    /// An addition.
    Add,
    /// An operation which has no effect.
    NoOp,
}

impl ToBytes for OpKind {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        match self {
            OpKind::Read => OP_READ_TAG.to_bytes(),
            OpKind::Write => OP_WRITE_TAG.to_bytes(),
            OpKind::Add => OP_ADD_TAG.to_bytes(),
            OpKind::NoOp => OP_NOOP_TAG.to_bytes(),
        }
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
    }
}

impl FromBytes for OpKind {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = u8::from_bytes(bytes)?;
        match tag {
            OP_READ_TAG => Ok((OpKind::Read, remainder)),
            OP_WRITE_TAG => Ok((OpKind::Write, remainder)),
            OP_ADD_TAG => Ok((OpKind::Add, remainder)),
            OP_NOOP_TAG => Ok((OpKind::NoOp, remainder)),
            _ => Err(bytesrepr::Error::Formatting),
        }
    }
}

/// A transformation performed while executing a deploy.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TransformEntry {
    /// The formatted string of the `Key`.
    pub key: String,
    /// The transformation.
    pub transform: Transform,
}

impl ToBytes for TransformEntry {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.key.to_bytes()?);
        buffer.extend(self.transform.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.key.serialized_length() + self.transform.serialized_length()
    }
}

impl FromBytes for TransformEntry {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (key, remainder) = String::from_bytes(bytes)?;
        let (transform, remainder) = Transform::from_bytes(remainder)?;
        let transform_entry = TransformEntry { key, transform };
        Ok((transform_entry, remainder))
    }
}

/// The actual transformation performed while executing a deploy.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub enum Transform {
    /// A transform having no effect.
    Identity,
    /// Writes the given CLValue to global state.
    WriteCLValue(CLValue),
    /// Writes the given Account to global state.
    WriteAccount(AccountHash),
    /// Writes a smart contract as Wasm to global state.
    WriteContractWasm,
    /// Writes a smart contract to global state.
    WriteContract,
    /// Writes a smart contract package to global state.
    WriteContractPackage,
    /// Writes the given DeployInfo to global state.
    WriteDeployInfo(DeployInfo),
    /// Writes the given EraInfo to global state.
    WriteEraInfo(EraInfo),
    /// Writes the given Transfer to global state.
    WriteTransfer(Transfer),
    /// Writes the given Bid to global state.
    WriteBid(Box<Bid>),
    /// Writes the given Withdraw to global state.
    WriteWithdraw(Vec<UnbondingPurse>),
    /// Adds the given `i32`.
    AddInt32(i32),
    /// Adds the given `u64`.
    AddUInt64(u64),
    /// Adds the given `U128`.
    AddUInt128(U128),
    /// Adds the given `U256`.
    AddUInt256(U256),
    /// Adds the given `U512`.
    AddUInt512(U512),
    /// Adds the given collection of named keys.
    AddKeys(Vec<NamedKey>),
    /// A failed transformation, containing an error message.
    Failure(String),
}

impl ToBytes for Transform {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        match self {
            Transform::Identity => buffer.insert(0, TRANSFORM_IDENTITY_TAG),
            Transform::WriteCLValue(value) => {
                buffer.insert(0, TRANSFORM_WRITE_CLVALUE_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::WriteAccount(account_hash) => {
                buffer.insert(0, TRANSFORM_WRITE_ACCOUNT_TAG);
                buffer.extend(account_hash.to_bytes()?);
            }
            Transform::WriteContractWasm => buffer.insert(0, TRANSFORM_WRITE_CONTRACT_WASM_TAG),
            Transform::WriteContract => buffer.insert(0, TRANSFORM_WRITE_CONTRACT_TAG),
            Transform::WriteContractPackage => {
                buffer.insert(0, TRANSFORM_WRITE_CONTRACT_PACKAGE_TAG)
            }
            Transform::WriteDeployInfo(deploy_info) => {
                buffer.insert(0, TRANSFORM_WRITE_DEPLOY_INFO_TAG);
                buffer.extend(deploy_info.to_bytes()?);
            }
            Transform::WriteEraInfo(era_info) => {
                buffer.insert(0, TRANSFORM_WRITE_ERA_INFO_TAG);
                buffer.extend(era_info.to_bytes()?);
            }
            Transform::WriteTransfer(transfer) => {
                buffer.insert(0, TRANSFORM_WRITE_TRANSFER_TAG);
                buffer.extend(transfer.to_bytes()?);
            }
            Transform::WriteBid(bid) => {
                buffer.insert(0, TRANSFORM_WRITE_BID_TAG);
                buffer.extend(bid.to_bytes()?);
            }
            Transform::WriteWithdraw(unbonding_purses) => {
                buffer.insert(0, TRANSFORM_WRITE_WITHDRAW_TAG);
                buffer.extend(unbonding_purses.to_bytes()?);
            }
            Transform::AddInt32(value) => {
                buffer.insert(0, TRANSFORM_ADD_INT32_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::AddUInt64(value) => {
                buffer.insert(0, TRANSFORM_ADD_UINT64_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::AddUInt128(value) => {
                buffer.insert(0, TRANSFORM_ADD_UINT128_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::AddUInt256(value) => {
                buffer.insert(0, TRANSFORM_ADD_UINT256_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::AddUInt512(value) => {
                buffer.insert(0, TRANSFORM_ADD_UINT512_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::AddKeys(value) => {
                buffer.insert(0, TRANSFORM_ADD_KEYS_TAG);
                buffer.extend(value.to_bytes()?);
            }
            Transform::Failure(value) => {
                buffer.insert(0, TRANSFORM_FAILURE_TAG);
                buffer.extend(value.to_bytes()?);
            }
        }
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        match self {
            Transform::WriteCLValue(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::WriteAccount(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::WriteDeployInfo(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::WriteEraInfo(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::WriteTransfer(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::AddInt32(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::AddUInt64(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::AddUInt128(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::AddUInt256(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::AddUInt512(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::AddKeys(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::Failure(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::Identity
            | Transform::WriteContractWasm
            | Transform::WriteContract
            | Transform::WriteContractPackage => U8_SERIALIZED_LENGTH,
            Transform::WriteBid(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
            Transform::WriteWithdraw(value) => value.serialized_length() + U8_SERIALIZED_LENGTH,
        }
    }
}

impl FromBytes for Transform {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = u8::from_bytes(bytes)?;
        match tag {
            TRANSFORM_IDENTITY_TAG => Ok((Transform::Identity, remainder)),
            TRANSFORM_WRITE_CLVALUE_TAG => {
                let (cl_value, remainder) = CLValue::from_bytes(remainder)?;
                Ok((Transform::WriteCLValue(cl_value), remainder))
            }
            TRANSFORM_WRITE_ACCOUNT_TAG => {
                let (account_hash, remainder) = AccountHash::from_bytes(remainder)?;
                Ok((Transform::WriteAccount(account_hash), remainder))
            }
            TRANSFORM_WRITE_CONTRACT_WASM_TAG => Ok((Transform::WriteContractWasm, remainder)),
            TRANSFORM_WRITE_CONTRACT_TAG => Ok((Transform::WriteContract, remainder)),
            TRANSFORM_WRITE_CONTRACT_PACKAGE_TAG => {
                Ok((Transform::WriteContractPackage, remainder))
            }
            TRANSFORM_WRITE_DEPLOY_INFO_TAG => {
                let (deploy_info, remainder) = DeployInfo::from_bytes(remainder)?;
                Ok((Transform::WriteDeployInfo(deploy_info), remainder))
            }
            TRANSFORM_WRITE_ERA_INFO_TAG => {
                let (era_info, remainder) = EraInfo::from_bytes(remainder)?;
                Ok((Transform::WriteEraInfo(era_info), remainder))
            }
            TRANSFORM_WRITE_TRANSFER_TAG => {
                let (transfer, remainder) = Transfer::from_bytes(remainder)?;
                Ok((Transform::WriteTransfer(transfer), remainder))
            }
            TRANSFORM_ADD_INT32_TAG => {
                let (value_i32, remainder) = i32::from_bytes(remainder)?;
                Ok((Transform::AddInt32(value_i32), remainder))
            }
            TRANSFORM_ADD_UINT64_TAG => {
                let (value_u64, remainder) = u64::from_bytes(remainder)?;
                Ok((Transform::AddUInt64(value_u64), remainder))
            }
            TRANSFORM_ADD_UINT128_TAG => {
                let (value_u128, remainder) = U128::from_bytes(remainder)?;
                Ok((Transform::AddUInt128(value_u128), remainder))
            }
            TRANSFORM_ADD_UINT256_TAG => {
                let (value_u256, remainder) = U256::from_bytes(remainder)?;
                Ok((Transform::AddUInt256(value_u256), remainder))
            }
            TRANSFORM_ADD_UINT512_TAG => {
                let (value_u512, remainder) = U512::from_bytes(remainder)?;
                Ok((Transform::AddUInt512(value_u512), remainder))
            }
            TRANSFORM_ADD_KEYS_TAG => {
                let (value, remainder) = Vec::<NamedKey>::from_bytes(remainder)?;
                Ok((Transform::AddKeys(value), remainder))
            }
            TRANSFORM_FAILURE_TAG => {
                let (value, remainder) = String::from_bytes(remainder)?;
                Ok((Transform::Failure(value), remainder))
            }
            _ => Err(bytesrepr::Error::Formatting),
        }
    }
}

impl Distribution<Transform> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Transform {
        // TODO - include WriteDeployInfo and WriteTransfer as options
        match rng.gen_range(0..13) {
            0 => Transform::Identity,
            1 => Transform::WriteCLValue(CLValue::from_t(true).unwrap()),
            2 => Transform::WriteAccount(AccountHash::new(rng.gen())),
            3 => Transform::WriteContractWasm,
            4 => Transform::WriteContract,
            5 => Transform::WriteContractPackage,
            6 => Transform::AddInt32(rng.gen()),
            7 => Transform::AddUInt64(rng.gen()),
            8 => Transform::AddUInt128(rng.gen::<u64>().into()),
            9 => Transform::AddUInt256(rng.gen::<u64>().into()),
            10 => Transform::AddUInt512(rng.gen::<u64>().into()),
            11 => {
                let mut named_keys = Vec::new();
                for _ in 0..rng.gen_range(1..6) {
                    let _ = named_keys.push(NamedKey {
                        name: rng.gen::<u64>().to_string(),
                        key: rng.gen::<u64>().to_string(),
                    });
                }
                Transform::AddKeys(named_keys)
            }
            12 => Transform::Failure(rng.gen::<u64>().to_string()),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::{rngs::SmallRng, Rng, SeedableRng};

    use super::*;

    fn get_rng() -> SmallRng {
        let mut seed = [0u8; 32];
        getrandom::getrandom(seed.as_mut()).unwrap();
        SmallRng::from_seed(seed)
    }

    #[test]
    fn bytesrepr_test_transform() {
        let mut rng = get_rng();
        let transform: Transform = rng.gen();
        bytesrepr::test_serialization_roundtrip(&transform);
    }

    #[test]
    fn bytesrepr_test_execution_result() {
        let mut rng = get_rng();
        let execution_result: ExecutionResult = rng.gen();
        bytesrepr::test_serialization_roundtrip(&execution_result);
    }
}
