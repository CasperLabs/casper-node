// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use datasize::DataSize;
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{rpcs::docs::DocExample, types::json_compatibility::vectorize};
use casper_execution_engine::shared::account::Account as ExecutionEngineAccount;
use casper_types::{account::AccountHash, NamedKey, PublicKey, SecretKey, URef};

static ACCOUNT: Lazy<Account> = Lazy::new(|| {
    let main_purse = URef::from_formatted_str(
        "uref-09480c3248ef76b603d386f3f4f8a5f87f597d4eaffd475433f861af187ab5db-007",
    )
    .unwrap();
    let secret_key = SecretKey::ed25519_from_bytes([0; 32]).unwrap();
    let account_hash = PublicKey::from(&secret_key).to_account_hash();
    let associated_key = AssociatedKey {
        account_hash,
        weight: 1u8,
    };
    let action_thresholds = ActionThresholds {
        deployment: 1u8,
        key_management: 1u8,
    };
    Account {
        account_hash,
        named_keys: vec![],
        main_purse,
        associated_keys: vec![associated_key],
        action_thresholds,
    }
});

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
                .iter()
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

impl DocExample for Account {
    fn doc_example() -> &'static Self {
        &*ACCOUNT
    }
}
