//! This file provides types to allow conversion from an EE `StoredValue` into a similar type
//! which can be serialized to a valid JSON representation.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::convert::{TryFrom, TryInto};

use hex::FromHexError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use casper_execution_engine::shared::stored_value::StoredValue as ExecutionEngineStoredValue;
use casper_types::{
    bytesrepr::{self, Bytes, ToBytes},
    system::auction::{Bid, EraInfo, UnbondingPurse},
    CLValue, ContractWasm, DeployInfo, Transfer,
};

use super::{Account, Contract, ContractPackage};
use crate::types::json_compatibility::{
    account::TryFromAccountForExecutionEngineAccountError,
    contracts::TryFromContractForDomainContract,
};

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
                StoredValue::ContractWasm(hex::encode(&contract_wasm.to_bytes()?))
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

#[derive(Error, Debug)]
pub enum TryFromStoredValueForExecutionEngineStoredValueError {
    #[error("could not convert from hex: {0}")]
    FromHexError(#[from] FromHexError),
    #[error("could not deserialize: {0}")]
    BytesreprError(#[from] bytesrepr::Error),
    #[error("could not convert JSON Account to domain object: {0}")]
    TryFromAccountForExecutionEngineAccountError(
        #[from] TryFromAccountForExecutionEngineAccountError,
    ),
    #[error("could not convert JSON Contract to domain object: {0}")]
    TryFromContractForDomainContract(#[from] TryFromContractForDomainContract),
}

impl TryFrom<StoredValue> for ExecutionEngineStoredValue {
    type Error = TryFromStoredValueForExecutionEngineStoredValueError;

    fn try_from(value: StoredValue) -> Result<Self, Self::Error> {
        match value {
            StoredValue::CLValue(cl_value) => {
                // No conversion necessary
                Ok(ExecutionEngineStoredValue::CLValue(cl_value))
            }
            StoredValue::Account(json_account) => {
                let account = json_account.try_into()?;
                Ok(ExecutionEngineStoredValue::Account(account))
            }
            StoredValue::ContractWasm(hex_bytes) => {
                let bytes = hex::decode(hex_bytes)?;
                let bytes_vec: Bytes = bytesrepr::deserialize(bytes)?;
                let contract_wasm = ContractWasm::new(bytes_vec.to_vec());
                Ok(ExecutionEngineStoredValue::ContractWasm(contract_wasm))
            }
            StoredValue::Contract(json_contract) => {
                let contract = json_contract.try_into()?;
                Ok(ExecutionEngineStoredValue::Contract(contract))
            }
            StoredValue::ContractPackage(json_contract_package) => {
                let contract_package = json_contract_package.into();
                Ok(ExecutionEngineStoredValue::ContractPackage(
                    contract_package,
                ))
            }
            StoredValue::Transfer(transfer) => {
                // No conversion necessary
                Ok(ExecutionEngineStoredValue::Transfer(transfer))
            }
            StoredValue::DeployInfo(deploy_info) => {
                // No conversion necessary
                Ok(ExecutionEngineStoredValue::DeployInfo(deploy_info))
            }
            StoredValue::EraInfo(era_info) => {
                // No conversion necessary
                Ok(ExecutionEngineStoredValue::EraInfo(era_info))
            }
            StoredValue::Bid(bid) => {
                // No conversion necessary
                Ok(ExecutionEngineStoredValue::Bid(bid))
            }
            StoredValue::Withdraw(withdraws) => {
                // No conversion necessary
                Ok(ExecutionEngineStoredValue::Withdraw(withdraws))
            }
        }
    }
}
