// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryFrom,
};

use datasize::DataSize;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::json_compatibility::vectorize;
use casper_types::{
    contracts::ContractPackageStatus, key::FromStrError, Contract as DomainContract, ContractHash,
    ContractPackage as DomainContractPackage, ContractPackageHash, ContractVersionKey,
    ContractWasmHash, EntryPoint, EntryPoints, Group, Key, NamedKey, ProtocolVersion, URef,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, DataSize, JsonSchema,
)]
pub struct ContractVersion {
    protocol_version_major: u32,
    contract_version: u32,
    contract_hash: ContractHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, DataSize, JsonSchema)]
pub struct DisabledVersion {
    protocol_version_major: u32,
    contract_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, DataSize, JsonSchema)]
pub struct Groups {
    group: String,
    #[data_size(skip)]
    keys: Vec<URef>,
}

/// A contract struct that can be serialized as  JSON object.
#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize, DataSize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    contract_package_hash: ContractPackageHash,
    contract_wasm_hash: ContractWasmHash,
    #[data_size(skip)]
    named_keys: Vec<NamedKey>,
    #[data_size(skip)]
    entry_points: Vec<EntryPoint>,
    #[data_size(skip)]
    #[schemars(with = "String")]
    protocol_version: ProtocolVersion,
}

impl From<&DomainContract> for Contract {
    fn from(contract: &DomainContract) -> Self {
        let entry_points = contract.entry_points().clone().take_entry_points();
        let named_keys = vectorize(contract.named_keys());
        Contract {
            contract_package_hash: contract.contract_package_hash(),
            contract_wasm_hash: contract.contract_wasm_hash(),
            named_keys,
            entry_points,
            protocol_version: contract.protocol_version(),
        }
    }
}

#[derive(Error, Debug)]
pub enum TryFromContractForDomainContract {
    #[error("could not convert from string: {0}")]
    FromStrError(#[from] FromStrError),
}

impl TryFrom<Contract> for DomainContract {
    type Error = TryFromContractForDomainContract;

    fn try_from(contract: Contract) -> Result<Self, Self::Error> {
        let contract_package_hash = contract.contract_package_hash;
        let contract_wasm_hash = contract.contract_wasm_hash;
        let named_keys = {
            let mut tmp = BTreeMap::new();
            let named_keys_vec = contract.named_keys;
            for named_key in named_keys_vec {
                let name = named_key.name;
                let key_str = named_key.key;
                let key: Key = Key::from_formatted_str(&key_str)?;
                tmp.insert(name, key);
            }
            tmp
        };
        let entry_points = {
            let entry_points_vec = contract.entry_points;
            EntryPoints::from(entry_points_vec)
        };
        let protocol_version = contract.protocol_version;
        Ok(DomainContract::new(
            contract_package_hash,
            contract_wasm_hash,
            named_keys,
            entry_points,
            protocol_version,
        ))
    }
}

/// Contract definition, metadata, and security container.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, DataSize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractPackage {
    #[data_size(skip)]
    access_key: URef,
    versions: Vec<ContractVersion>,
    disabled_versions: Vec<DisabledVersion>,
    groups: Vec<Groups>,
    lock_status: ContractPackageStatus,
}

impl From<&DomainContractPackage> for ContractPackage {
    fn from(contract_package: &DomainContractPackage) -> Self {
        let versions = contract_package
            .versions()
            .iter()
            .map(|(version_key, hash)| ContractVersion {
                protocol_version_major: version_key.protocol_version_major(),
                contract_version: version_key.contract_version(),
                contract_hash: *hash,
            })
            .collect();

        let disabled_versions = contract_package
            .disabled_versions()
            .iter()
            .map(|version| DisabledVersion {
                protocol_version_major: version.protocol_version_major(),
                contract_version: version.contract_version(),
            })
            .collect();

        let groups = contract_package
            .groups()
            .iter()
            .map(|(group, keys)| Groups {
                group: group.clone().value().to_string(),
                keys: keys.iter().cloned().collect(),
            })
            .collect();

        ContractPackage {
            access_key: contract_package.access_key(),
            versions,
            disabled_versions,
            groups,
            lock_status: contract_package.lock_status(),
        }
    }
}

impl From<ContractPackage> for DomainContractPackage {
    fn from(contract_package: ContractPackage) -> Self {
        let access_key = contract_package.access_key;
        let versions = {
            let mut tmp = BTreeMap::new();
            for version in contract_package.versions {
                let contract_version_key = ContractVersionKey::new(
                    version.protocol_version_major,
                    version.contract_version,
                );
                tmp.insert(contract_version_key, version.contract_hash);
            }
            tmp
        };
        let disabled_versions = {
            let mut tmp = BTreeSet::new();
            for disabled_version in contract_package.disabled_versions {
                let contract_version_key = ContractVersionKey::new(
                    disabled_version.protocol_version_major,
                    disabled_version.contract_version,
                );
                tmp.insert(contract_version_key);
            }
            tmp
        };
        let groups = {
            let mut tmp = BTreeMap::new();
            for group in contract_package.groups {
                let domain_group = Group::new(group.group);
                let keys: BTreeSet<URef> = group.keys.into_iter().collect();
                tmp.insert(domain_group, keys);
            }
            tmp
        };
        let lock_status = contract_package.lock_status;
        DomainContractPackage::new(access_key, versions, disabled_versions, groups, lock_status)
    }
}
