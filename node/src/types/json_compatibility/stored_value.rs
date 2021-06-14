//! This file provides types to allow conversion from an EE `StoredValue` into a similar type
//! which can be serialized to a valid JSON representation.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::convert::TryFrom;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use casper_execution_engine::shared::stored_value::StoredValue as ExecutionEngineStoredValue;
use casper_types::{
    bytesrepr::{self, ToBytes},
    check_summed_hex,
    system::auction::{Bid, EraInfo, UnbondingPurse},
    CLValue, DeployInfo, Transfer,
};

use super::{Account, Contract, ContractPackage};

/// Representation of a value stored in global state.
///
/// `Account`, `Contract` and `ContractPackage` have their own `json_compatibility` representations
/// (see their docs for further info).
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum StoredValue {
    /// A CasperLabs value.
    CLValue(CLValue),
    /// An account.
    Account(Account),
    /// A contract's Wasm
    ContractWasm(String),
    /// Methods and type signatures supported by a contract.
    Contract(Contract),
    /// A contract definition, metadata, and security container.
    ContractPackage(ContractPackage),
    /// A record of a transfer
    Transfer(Transfer),
    /// A record of a deploy
    DeployInfo(DeployInfo),
    /// Auction metadata
    EraInfo(EraInfo),
    /// A bid
    Bid(Box<Bid>),
    /// A withdraw
    Withdraw(Vec<UnbondingPurse>),
}

impl TryFrom<&ExecutionEngineStoredValue> for StoredValue {
    type Error = bytesrepr::Error;

    fn try_from(ee_stored_value: &ExecutionEngineStoredValue) -> Result<Self, Self::Error> {
        let stored_value = match ee_stored_value {
            ExecutionEngineStoredValue::CLValue(cl_value) => StoredValue::CLValue(cl_value.clone()),
            ExecutionEngineStoredValue::Account(account) => StoredValue::Account(account.into()),
            ExecutionEngineStoredValue::ContractWasm(contract_wasm) => {
                StoredValue::ContractWasm(check_summed_hex::encode(&contract_wasm.to_bytes()?))
            }
            ExecutionEngineStoredValue::Contract(contract) => {
                StoredValue::Contract(contract.into())
            }
            ExecutionEngineStoredValue::ContractPackage(contract_package) => {
                StoredValue::ContractPackage(contract_package.into())
            }
            ExecutionEngineStoredValue::Transfer(transfer) => StoredValue::Transfer(*transfer),
            ExecutionEngineStoredValue::DeployInfo(deploy_info) => {
                StoredValue::DeployInfo(deploy_info.clone())
            }
            ExecutionEngineStoredValue::EraInfo(era_info) => StoredValue::EraInfo(era_info.clone()),
            ExecutionEngineStoredValue::Bid(bid) => StoredValue::Bid(bid.clone()),
            ExecutionEngineStoredValue::Withdraw(unbonding_purses) => {
                StoredValue::Withdraw(unbonding_purses.clone())
            }
        };

        Ok(stored_value)
    }
}
