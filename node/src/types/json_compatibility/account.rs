// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::{collections::BTreeMap, convert::TryFrom};

use datasize::DataSize;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use casper_execution_engine::shared::account::{
    Account as ExecutionEngineAccount, ActionThresholds as ExecutionEngineActionThresholds,
    AssociatedKeys,
};
use casper_types::{
    account::{AccountHash, SetThresholdFailure, Weight},
    key::FromStrError,
    Key, NamedKey, URef,
};

use crate::types::json_compatibility::vectorize;

#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize, DataSize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssociatedKey {
    account_hash: AccountHash,
    weight: u8,
}

/// Thresholds that have to be met when executing an action of a certain type.
#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize, DataSize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ActionThresholds {
    deployment: u8,
    key_management: u8,
}

/// Structure representing a user's account, stored in global state.
#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize, DataSize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Account {
    account_hash: AccountHash,
    #[data_size(skip)]
    named_keys: Vec<NamedKey>,
    #[data_size(skip)]
    main_purse: URef,
    associated_keys: Vec<AssociatedKey>,
    action_thresholds: ActionThresholds,
}

impl From<&ExecutionEngineAccount> for Account {
    fn from(ee_account: &ExecutionEngineAccount) -> Self {
        Account {
            account_hash: ee_account.account_hash(),
            named_keys: vectorize(ee_account.named_keys()),
            main_purse: ee_account.main_purse(),
            associated_keys: ee_account
                .associated_keys()
                .map(|(account_hash, weight)| AssociatedKey {
                    account_hash: *account_hash,
                    weight: weight.value(),
                })
                .collect(),
            action_thresholds: ActionThresholds {
                deployment: ee_account.action_thresholds().deployment().value(),
                key_management: ee_account.action_thresholds().key_management().value(),
            },
        }
    }
}

#[derive(Error, Debug)]
pub enum TryFromAccountForExecutionEngineAccountError {
    #[error("could not convert from string: {0}")]
    FromStrError(#[from] FromStrError),
    #[error("could not set action thresholds: {0}")]
    SetThresholdFailure(#[from] SetThresholdFailure),
}

impl TryFrom<Account> for ExecutionEngineAccount {
    type Error = TryFromAccountForExecutionEngineAccountError;

    fn try_from(account: Account) -> Result<Self, Self::Error> {
        let account_hash = account.account_hash;
        let named_keys = {
            let mut tmp = BTreeMap::new();
            for named_key in account.named_keys {
                let name = named_key.name;
                let key_str = named_key.key;
                let key: Key = Key::from_formatted_str(&key_str)?;
                tmp.insert(name, key);
            }
            tmp
        };
        let main_purse = account.main_purse;
        let associated_keys = {
            let mut tmp = BTreeMap::new();
            for associated_key in account.associated_keys {
                let account_hash = associated_key.account_hash;
                let weight = Weight::new(associated_key.weight);
                tmp.insert(account_hash, weight);
            }
            AssociatedKeys::from(tmp)
        };
        let action_thresholds = ExecutionEngineActionThresholds::new(
            Weight::new(account.action_thresholds.deployment),
            Weight::new(account.action_thresholds.key_management),
        )?;
        Ok(ExecutionEngineAccount::new(
            account_hash,
            named_keys,
            main_purse,
            associated_keys,
            action_thresholds,
        ))
    }
}
