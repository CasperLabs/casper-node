// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::{
    cell::RefCell,
    fmt::{self, Debug, Display, Formatter},
    rc::Rc,
};

use datasize::DataSize;
use parity_wasm::elements::Module;
use rand::{
    distributions::{Alphanumeric, Distribution, Standard},
    Rng,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use casper_types::{
    bytesrepr::{self, Bytes, FromBytes, ToBytes, U8_SERIALIZED_LENGTH},
    check_summed_hex::{self, CheckSummedHex, CheckSummedHexForm},
    contracts::{ContractVersion, DEFAULT_ENTRY_POINT_NAME},
    system::mint::ARG_AMOUNT,
    CLValue, Contract, ContractHash, ContractPackage, ContractPackageHash, ContractVersionKey,
    EntryPoint, EntryPointType, Key, Phase, ProtocolVersion, RuntimeArgs, U512,
};

use super::error;
use crate::{
    core::{
        engine_state::{Error, ExecError, MAX_PAYMENT_AMOUNT},
        execution,
        tracking_copy::{TrackingCopy, TrackingCopyExt},
    },
    shared::{
        account::Account, newtypes::CorrelationId, stored_value::StoredValue, wasm_prep,
        wasm_prep::Preprocessor,
    },
    storage::{global_state::StateReader, protocol_data::ProtocolData},
};

const TAG_LENGTH: usize = U8_SERIALIZED_LENGTH;
const MODULE_BYTES_TAG: u8 = 0;
const STORED_CONTRACT_BY_HASH_TAG: u8 = 1;
const STORED_CONTRACT_BY_NAME_TAG: u8 = 2;
const STORED_VERSIONED_CONTRACT_BY_HASH_TAG: u8 = 3;
const STORED_VERSIONED_CONTRACT_BY_NAME_TAG: u8 = 4;
const TRANSFER_TAG: u8 = 5;

#[derive(
    Clone, DataSize, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub enum ExecutableDeployItem {
    ModuleBytes {
        #[serde(with = "CheckSummedHexForm")]
        #[schemars(
            with = "String",
            description = "Check-summed hex-encoded raw Wasm bytes."
        )]
        module_bytes: Bytes,
        // assumes implicit `call` noarg entrypoint
        args: RuntimeArgs,
    },
    StoredContractByHash {
        #[serde(with = "CheckSummedHexForm")]
        #[schemars(with = "String", description = "Check-summed hex-encoded hash.")]
        hash: ContractHash,
        entry_point: String,
        args: RuntimeArgs,
    },
    StoredContractByName {
        name: String,
        entry_point: String,
        args: RuntimeArgs,
    },
    StoredVersionedContractByHash {
        #[serde(with = "CheckSummedHexForm")]
        #[schemars(with = "String", description = "Check-summed hex-encoded hash.")]
        hash: ContractPackageHash,
        version: Option<ContractVersion>, // defaults to highest enabled version
        entry_point: String,
        args: RuntimeArgs,
    },
    StoredVersionedContractByName {
        name: String,
        version: Option<ContractVersion>, // defaults to highest enabled version
        entry_point: String,
        args: RuntimeArgs,
    },
    Transfer {
        args: RuntimeArgs,
    },
}

impl ExecutableDeployItem {
    pub(crate) fn to_contract_hash_key(&self, account: &Account) -> Result<Option<Key>, Error> {
        match self {
            ExecutableDeployItem::StoredContractByHash { hash, .. } => {
                Ok(Some(Key::from(hash.value())))
            }
            ExecutableDeployItem::StoredVersionedContractByHash { hash, .. } => {
                Ok(Some(Key::from(hash.value())))
            }
            ExecutableDeployItem::StoredContractByName { name, .. }
            | ExecutableDeployItem::StoredVersionedContractByName { name, .. } => {
                let key = account.named_keys().get(name).cloned().ok_or_else(|| {
                    error::Error::Exec(execution::Error::NamedKeyNotFound(name.to_string()))
                })?;
                Ok(Some(key))
            }
            ExecutableDeployItem::ModuleBytes { .. } | ExecutableDeployItem::Transfer { .. } => {
                Ok(None)
            }
        }
    }

    pub fn entry_point_name(&self) -> &str {
        match self {
            ExecutableDeployItem::ModuleBytes { .. } | ExecutableDeployItem::Transfer { .. } => {
                DEFAULT_ENTRY_POINT_NAME
            }
            ExecutableDeployItem::StoredVersionedContractByName { entry_point, .. }
            | ExecutableDeployItem::StoredVersionedContractByHash { entry_point, .. }
            | ExecutableDeployItem::StoredContractByHash { entry_point, .. }
            | ExecutableDeployItem::StoredContractByName { entry_point, .. } => &entry_point,
        }
    }

    pub fn args(&self) -> &RuntimeArgs {
        match self {
            ExecutableDeployItem::ModuleBytes { args, .. }
            | ExecutableDeployItem::StoredContractByHash { args, .. }
            | ExecutableDeployItem::StoredContractByName { args, .. }
            | ExecutableDeployItem::StoredVersionedContractByHash { args, .. }
            | ExecutableDeployItem::StoredVersionedContractByName { args, .. }
            | ExecutableDeployItem::Transfer { args } => args,
        }
    }

    pub fn is_transfer(&self) -> bool {
        matches!(self, ExecutableDeployItem::Transfer { .. })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_deploy_metadata<R>(
        &self,
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        account: &Account,
        correlation_id: CorrelationId,
        preprocessor: &Preprocessor,
        protocol_version: &ProtocolVersion,
        protocol_data: &ProtocolData,
        phase: Phase,
    ) -> Result<DeployMetadata, Error>
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<ExecError>,
    {
        let (contract_package, contract, contract_hash, base_key) = match self {
            ExecutableDeployItem::ModuleBytes { module_bytes, .. } => {
                let is_payment = matches!(phase, Phase::Payment);
                if is_payment && module_bytes.is_empty() {
                    return Ok(DeployMetadata::System {
                        base_key: account.account_hash().into(),
                        contract: Contract::default(),
                        contract_package: ContractPackage::default(),
                        entry_point: EntryPoint::default(),
                    });
                }

                let module = preprocessor.preprocess(&module_bytes.as_ref())?;
                return Ok(DeployMetadata::Session {
                    module,
                    contract_package: ContractPackage::default(),
                    entry_point: EntryPoint::default(),
                });
            }
            ExecutableDeployItem::StoredContractByHash { .. }
            | ExecutableDeployItem::StoredContractByName { .. } => {
                // NOTE: `to_contract_hash_key` ensures it returns valid value only for
                // ByHash/ByName variants
                let stored_contract_key = self.to_contract_hash_key(&account)?.unwrap();

                let contract_hash = stored_contract_key
                    .into_hash()
                    .ok_or(Error::InvalidKeyVariant)?;

                let contract = tracking_copy
                    .borrow_mut()
                    .get_contract(correlation_id, contract_hash.into())?;

                if !contract.is_compatible_protocol_version(*protocol_version) {
                    let exec_error = execution::Error::IncompatibleProtocolMajorVersion {
                        expected: protocol_version.value().major,
                        actual: contract.protocol_version().value().major,
                    };
                    return Err(error::Error::Exec(exec_error));
                }

                let contract_package = tracking_copy
                    .borrow_mut()
                    .get_contract_package(correlation_id, contract.contract_package_hash())?;

                (
                    contract_package,
                    contract,
                    contract_hash,
                    stored_contract_key,
                )
            }
            ExecutableDeployItem::StoredVersionedContractByName { version, .. }
            | ExecutableDeployItem::StoredVersionedContractByHash { version, .. } => {
                // NOTE: `to_contract_hash_key` ensures it returns valid value only for
                // ByHash/ByName variants
                let contract_package_key = self.to_contract_hash_key(&account)?.unwrap();
                let contract_package_hash = contract_package_key
                    .into_hash()
                    .ok_or(Error::InvalidKeyVariant)?;

                let contract_package = tracking_copy
                    .borrow_mut()
                    .get_contract_package(correlation_id, contract_package_hash.into())?;

                let maybe_version_key =
                    version.map(|ver| ContractVersionKey::new(protocol_version.value().major, ver));

                let contract_version_key = maybe_version_key
                    .or_else(|| contract_package.current_contract_version())
                    .ok_or_else(|| {
                        error::Error::Exec(execution::Error::NoActiveContractVersions(
                            contract_package_hash.into(),
                        ))
                    })?;

                if !contract_package.is_version_enabled(contract_version_key) {
                    return Err(error::Error::Exec(
                        execution::Error::InvalidContractVersion(contract_version_key),
                    ));
                }

                let contract_hash = *contract_package
                    .lookup_contract_hash(contract_version_key)
                    .ok_or(error::Error::Exec(
                        execution::Error::InvalidContractVersion(contract_version_key),
                    ))?;

                let contract = tracking_copy
                    .borrow_mut()
                    .get_contract(correlation_id, contract_hash)?;

                (
                    contract_package,
                    contract,
                    contract_hash.value(),
                    contract_package_key,
                )
            }
            ExecutableDeployItem::Transfer { .. } => {
                return Err(error::Error::InvalidDeployItemVariant(String::from(
                    "Transfer",
                )))
            }
        };

        let entry_point_name = self.entry_point_name();

        let entry_point = contract
            .entry_point(entry_point_name)
            .cloned()
            .ok_or_else(|| {
                error::Error::Exec(execution::Error::NoSuchMethod(entry_point_name.to_owned()))
            })?;

        if protocol_data
            .system_contracts()
            .contains(&contract_hash.into())
        {
            return Ok(DeployMetadata::System {
                base_key,
                contract,
                contract_package,
                entry_point,
            });
        }

        let contract_wasm = tracking_copy
            .borrow_mut()
            .get_contract_wasm(correlation_id, contract.contract_wasm_hash())?;

        let module = wasm_prep::deserialize(contract_wasm.bytes())?;

        match entry_point.entry_point_type() {
            EntryPointType::Session => Ok(DeployMetadata::Session {
                module,
                contract_package,
                entry_point,
            }),
            EntryPointType::Contract => Ok(DeployMetadata::Contract {
                module,
                base_key,
                contract,
                contract_package,
                entry_point,
            }),
        }
    }
}

impl ToBytes for ExecutableDeployItem {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        match self {
            ExecutableDeployItem::ModuleBytes { module_bytes, args } => {
                buffer.insert(0, MODULE_BYTES_TAG);
                buffer.extend(module_bytes.to_bytes()?);
                buffer.extend(args.to_bytes()?);
            }
            ExecutableDeployItem::StoredContractByHash {
                hash,
                entry_point,
                args,
            } => {
                buffer.insert(0, STORED_CONTRACT_BY_HASH_TAG);
                buffer.extend(hash.to_bytes()?);
                buffer.extend(entry_point.to_bytes()?);
                buffer.extend(args.to_bytes()?)
            }
            ExecutableDeployItem::StoredContractByName {
                name,
                entry_point,
                args,
            } => {
                buffer.insert(0, STORED_CONTRACT_BY_NAME_TAG);
                buffer.extend(name.to_bytes()?);
                buffer.extend(entry_point.to_bytes()?);
                buffer.extend(args.to_bytes()?)
            }
            ExecutableDeployItem::StoredVersionedContractByHash {
                hash,
                version,
                entry_point,
                args,
            } => {
                buffer.insert(0, STORED_VERSIONED_CONTRACT_BY_HASH_TAG);
                buffer.extend(hash.to_bytes()?);
                buffer.extend(version.to_bytes()?);
                buffer.extend(entry_point.to_bytes()?);
                buffer.extend(args.to_bytes()?)
            }
            ExecutableDeployItem::StoredVersionedContractByName {
                name,
                version,
                entry_point,
                args,
            } => {
                buffer.insert(0, STORED_VERSIONED_CONTRACT_BY_NAME_TAG);
                buffer.extend(name.to_bytes()?);
                buffer.extend(version.to_bytes()?);
                buffer.extend(entry_point.to_bytes()?);
                buffer.extend(args.to_bytes()?)
            }
            ExecutableDeployItem::Transfer { args } => {
                buffer.insert(0, TRANSFER_TAG);
                buffer.extend(args.to_bytes()?)
            }
        }
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        TAG_LENGTH
            + match self {
                ExecutableDeployItem::ModuleBytes { module_bytes, args } => {
                    module_bytes.serialized_length() + args.serialized_length()
                }
                ExecutableDeployItem::StoredContractByHash {
                    hash,
                    entry_point,
                    args,
                } => {
                    hash.serialized_length()
                        + entry_point.serialized_length()
                        + args.serialized_length()
                }
                ExecutableDeployItem::StoredContractByName {
                    name,
                    entry_point,
                    args,
                } => {
                    name.serialized_length()
                        + entry_point.serialized_length()
                        + args.serialized_length()
                }
                ExecutableDeployItem::StoredVersionedContractByHash {
                    hash,
                    version,
                    entry_point,
                    args,
                } => {
                    hash.serialized_length()
                        + version.serialized_length()
                        + entry_point.serialized_length()
                        + args.serialized_length()
                }
                ExecutableDeployItem::StoredVersionedContractByName {
                    name,
                    version,
                    entry_point,
                    args,
                } => {
                    name.serialized_length()
                        + version.serialized_length()
                        + entry_point.serialized_length()
                        + args.serialized_length()
                }
                ExecutableDeployItem::Transfer { args } => args.serialized_length(),
            }
    }
}

impl FromBytes for ExecutableDeployItem {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = u8::from_bytes(bytes)?;
        match tag {
            MODULE_BYTES_TAG => {
                let (module_bytes, remainder) = FromBytes::from_bytes(remainder)?;
                let (args, remainder) = FromBytes::from_bytes(remainder)?;
                Ok((
                    ExecutableDeployItem::ModuleBytes { module_bytes, args },
                    remainder,
                ))
            }
            STORED_CONTRACT_BY_HASH_TAG => {
                let (hash, remainder) = FromBytes::from_bytes(remainder)?;
                let (entry_point, remainder) = String::from_bytes(remainder)?;
                let (args, remainder) = FromBytes::from_bytes(remainder)?;
                Ok((
                    ExecutableDeployItem::StoredContractByHash {
                        hash,
                        entry_point,
                        args,
                    },
                    remainder,
                ))
            }
            STORED_CONTRACT_BY_NAME_TAG => {
                let (name, remainder) = String::from_bytes(remainder)?;
                let (entry_point, remainder) = String::from_bytes(remainder)?;
                let (args, remainder) = FromBytes::from_bytes(remainder)?;
                Ok((
                    ExecutableDeployItem::StoredContractByName {
                        name,
                        entry_point,
                        args,
                    },
                    remainder,
                ))
            }
            STORED_VERSIONED_CONTRACT_BY_HASH_TAG => {
                let (hash, remainder) = FromBytes::from_bytes(remainder)?;
                let (version, remainder) = Option::<ContractVersion>::from_bytes(remainder)?;
                let (entry_point, remainder) = String::from_bytes(remainder)?;
                let (args, remainder) = FromBytes::from_bytes(remainder)?;
                Ok((
                    ExecutableDeployItem::StoredVersionedContractByHash {
                        hash,
                        version,
                        entry_point,
                        args,
                    },
                    remainder,
                ))
            }
            STORED_VERSIONED_CONTRACT_BY_NAME_TAG => {
                let (name, remainder) = String::from_bytes(remainder)?;
                let (version, remainder) = Option::<ContractVersion>::from_bytes(remainder)?;
                let (entry_point, remainder) = String::from_bytes(remainder)?;
                let (args, remainder) = FromBytes::from_bytes(remainder)?;
                Ok((
                    ExecutableDeployItem::StoredVersionedContractByName {
                        name,
                        version,
                        entry_point,
                        args,
                    },
                    remainder,
                ))
            }
            TRANSFER_TAG => {
                let (args, remainder) = FromBytes::from_bytes(remainder)?;
                Ok((ExecutableDeployItem::Transfer { args }, remainder))
            }
            _ => Err(bytesrepr::Error::Formatting),
        }
    }
}

impl Display for ExecutableDeployItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ExecutableDeployItem::ModuleBytes { module_bytes, .. } => {
                write!(f, "module-bytes [{} bytes]", module_bytes.len())
            }
            ExecutableDeployItem::StoredContractByHash {
                hash, entry_point, ..
            } => write!(
                f,
                "stored-contract-by-hash: {:10}, entry-point: {}",
                check_summed_hex::encode(hash),
                entry_point,
            ),
            ExecutableDeployItem::StoredContractByName {
                name, entry_point, ..
            } => write!(
                f,
                "stored-contract-by-name: {}, entry-point: {}",
                name, entry_point,
            ),
            ExecutableDeployItem::StoredVersionedContractByHash {
                hash,
                version: Some(ver),
                entry_point,
                ..
            } => write!(
                f,
                "stored-versioned-contract-by-hash: {:10}, version: {}, entry-point: {}",
                check_summed_hex::encode(hash),
                ver,
                entry_point,
            ),
            ExecutableDeployItem::StoredVersionedContractByHash {
                hash, entry_point, ..
            } => write!(
                f,
                "stored-versioned-contract-by-hash: {:10}, version: latest, entry-point: {}",
                check_summed_hex::encode(hash),
                entry_point,
            ),
            ExecutableDeployItem::StoredVersionedContractByName {
                name,
                version: Some(ver),
                entry_point,
                ..
            } => write!(
                f,
                "stored-versioned-contract: {}, version: {}, entry-point: {}",
                name, ver, entry_point,
            ),
            ExecutableDeployItem::StoredVersionedContractByName {
                name, entry_point, ..
            } => write!(
                f,
                "stored-versioned-contract: {}, version: latest, entry-point: {}",
                name, entry_point,
            ),
            ExecutableDeployItem::Transfer { .. } => write!(f, "transfer"),
        }
    }
}

impl Debug for ExecutableDeployItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ExecutableDeployItem::ModuleBytes { module_bytes, args } => f
                .debug_struct("ModuleBytes")
                .field("module_bytes", &format!("[{} bytes]", module_bytes.len()))
                .field("args", args)
                .finish(),
            ExecutableDeployItem::StoredContractByHash {
                hash,
                entry_point,
                args,
            } => f
                .debug_struct("StoredContractByHash")
                .field("hash", &check_summed_hex::encode(hash))
                .field("entry_point", &entry_point)
                .field("args", args)
                .finish(),
            ExecutableDeployItem::StoredContractByName {
                name,
                entry_point,
                args,
            } => f
                .debug_struct("StoredContractByName")
                .field("name", &name)
                .field("entry_point", &entry_point)
                .field("args", args)
                .finish(),
            ExecutableDeployItem::StoredVersionedContractByHash {
                hash,
                version,
                entry_point,
                args,
            } => f
                .debug_struct("StoredVersionedContractByHash")
                .field("hash", &check_summed_hex::encode(hash))
                .field("version", version)
                .field("entry_point", &entry_point)
                .field("args", args)
                .finish(),
            ExecutableDeployItem::StoredVersionedContractByName {
                name,
                version,
                entry_point,
                args,
            } => f
                .debug_struct("StoredVersionedContractByName")
                .field("name", &name)
                .field("version", version)
                .field("entry_point", &entry_point)
                .field("args", args)
                .finish(),
            ExecutableDeployItem::Transfer { args } => {
                f.debug_struct("Transfer").field("args", args).finish()
            }
        }
    }
}

impl Distribution<ExecutableDeployItem> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ExecutableDeployItem {
        fn random_bytes<R: Rng + ?Sized>(rng: &mut R) -> Vec<u8> {
            let mut bytes = vec![0u8; rng.gen_range(0..100)];
            rng.fill_bytes(bytes.as_mut());
            bytes
        }

        fn random_string<R: Rng + ?Sized>(rng: &mut R) -> String {
            rng.sample_iter(&Alphanumeric)
                .take(20)
                .map(char::from)
                .collect()
        }

        let mut args = RuntimeArgs::new();
        let _ = args.insert(random_string(rng), Bytes::from(random_bytes(rng)));

        match rng.gen_range(0..5) {
            0 => ExecutableDeployItem::ModuleBytes {
                module_bytes: random_bytes(rng).into(),
                args,
            },
            1 => ExecutableDeployItem::StoredContractByHash {
                hash: ContractHash::new(rng.gen()),
                entry_point: random_string(rng),
                args,
            },
            2 => ExecutableDeployItem::StoredContractByName {
                name: random_string(rng),
                entry_point: random_string(rng),
                args,
            },
            3 => ExecutableDeployItem::StoredVersionedContractByHash {
                hash: ContractPackageHash::new(rng.gen()),
                version: rng.gen(),
                entry_point: random_string(rng),
                args,
            },
            4 => ExecutableDeployItem::StoredVersionedContractByName {
                name: random_string(rng),
                version: rng.gen(),
                entry_point: random_string(rng),
                args,
            },
            5 => {
                let amount = rng.gen_range(MAX_PAYMENT_AMOUNT..1_000_000_000_000_000);
                let mut transfer_args = RuntimeArgs::new();
                transfer_args.insert_cl_value(
                    ARG_AMOUNT,
                    CLValue::from_t(U512::from(amount)).expect("should get CLValue from U512"),
                );
                ExecutableDeployItem::Transfer {
                    args: transfer_args,
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum DeployMetadata {
    Session {
        module: Module,
        contract_package: ContractPackage,
        entry_point: EntryPoint,
    },
    Contract {
        // Contract hash
        base_key: Key,
        module: Module,
        contract: Contract,
        contract_package: ContractPackage,
        entry_point: EntryPoint,
    },
    System {
        base_key: Key,
        contract: Contract,
        contract_package: ContractPackage,
        entry_point: EntryPoint,
    },
}

impl DeployMetadata {
    pub fn take_module(self) -> Option<Module> {
        match self {
            DeployMetadata::System { .. } => None,
            DeployMetadata::Session { module, .. } => Some(module),
            DeployMetadata::Contract { module, .. } => Some(module),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialization_roundtrip() {
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let executable_deploy_item: ExecutableDeployItem = rng.gen();
            bytesrepr::test_serialization_roundtrip(&executable_deploy_item);
        }
    }
}
