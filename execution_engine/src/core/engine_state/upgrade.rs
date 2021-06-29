use std::{cell::RefCell, collections::BTreeMap, fmt, rc::Rc};

use num_rational::Ratio;
use thiserror::Error;

use casper_types::{
    bytesrepr,
    system::{
        auction, handle_payment, mint, standard_payment, AUCTION, HANDLE_PAYMENT, MINT,
        STANDARD_PAYMENT,
    },
    Contract, ContractHash, EntryPoints, EraId, Key, ProtocolVersion,
};

use crate::{
    core::{engine_state::execution_effect::ExecutionEffect, tracking_copy::TrackingCopy},
    shared::{
        core_config::CoreConfig,
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
        system_costs::SystemCosts,
        wasm_config::WasmConfig,
        TypeMismatch,
    },
    storage::{
        global_state::{CommitResult, StateProvider},
        protocol_data::ProtocolData,
    },
};

#[derive(Debug, Clone)]
pub enum UpgradeResult {
    RootNotFound,
    KeyNotFound(Key),
    TypeMismatch(TypeMismatch),
    Serialization(bytesrepr::Error),
    Success {
        post_state_hash: Blake2bHash,
        effect: ExecutionEffect,
    },
}

impl fmt::Display for UpgradeResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            UpgradeResult::RootNotFound => write!(f, "Root not found"),
            UpgradeResult::KeyNotFound(key) => write!(f, "Key not found: {}", key),
            UpgradeResult::TypeMismatch(type_mismatch) => {
                write!(f, "Type mismatch: {:?}", type_mismatch)
            }
            UpgradeResult::Serialization(error) => write!(f, "Serialization error: {:?}", error),
            UpgradeResult::Success {
                post_state_hash,
                effect,
            } => write!(f, "Success: {} {:?}", post_state_hash, effect),
        }
    }
}

impl UpgradeResult {
    pub fn from_commit_result(commit_result: CommitResult, effect: ExecutionEffect) -> Self {
        match commit_result {
            CommitResult::RootNotFound => UpgradeResult::RootNotFound,
            CommitResult::KeyNotFound(key) => UpgradeResult::KeyNotFound(key),
            CommitResult::TypeMismatch(type_mismatch) => UpgradeResult::TypeMismatch(type_mismatch),
            CommitResult::Serialization(error) => UpgradeResult::Serialization(error),
            CommitResult::Success { state_root, .. } => UpgradeResult::Success {
                post_state_hash: state_root,
                effect,
            },
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(&self, UpgradeResult::Success { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeConfig {
    pre_state_hash: Blake2bHash,
    current_protocol_version: ProtocolVersion,
    new_protocol_version: ProtocolVersion,
    wasm_config: Option<WasmConfig>,
    core_config: Option<CoreConfig>,
    system_costs: Option<SystemCosts>,
    activation_point: Option<EraId>,
    new_validator_slots: Option<u32>,
    new_auction_delay: Option<u64>,
    new_locked_funds_period_millis: Option<u64>,
    new_round_seigniorage_rate: Option<Ratio<u64>>,
    new_unbonding_delay: Option<u64>,
    global_state_update: BTreeMap<Key, StoredValue>,
}

impl UpgradeConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pre_state_hash: Blake2bHash,
        current_protocol_version: ProtocolVersion,
        new_protocol_version: ProtocolVersion,
        wasm_config: Option<WasmConfig>,
        core_config: Option<CoreConfig>,
        system_costs: Option<SystemCosts>,
        activation_point: Option<EraId>,
        new_validator_slots: Option<u32>,
        new_auction_delay: Option<u64>,
        new_locked_funds_period_millis: Option<u64>,
        new_round_seigniorage_rate: Option<Ratio<u64>>,
        new_unbonding_delay: Option<u64>,
        global_state_update: BTreeMap<Key, StoredValue>,
    ) -> Self {
        UpgradeConfig {
            pre_state_hash,
            current_protocol_version,
            new_protocol_version,
            wasm_config,
            core_config,
            system_costs,
            activation_point,
            new_validator_slots,
            new_auction_delay,
            new_locked_funds_period_millis,
            new_round_seigniorage_rate,
            new_unbonding_delay,
            global_state_update,
        }
    }

    pub fn pre_state_hash(&self) -> Blake2bHash {
        self.pre_state_hash
    }

    pub fn current_protocol_version(&self) -> ProtocolVersion {
        self.current_protocol_version
    }

    pub fn new_protocol_version(&self) -> ProtocolVersion {
        self.new_protocol_version
    }

    pub fn wasm_config(&self) -> Option<&WasmConfig> {
        self.wasm_config.as_ref()
    }

    pub fn core_config(&self) -> Option<&CoreConfig> {
        self.core_config.as_ref()
    }

    pub fn system_costs(&self) -> Option<&SystemCosts> {
        self.system_costs.as_ref()
    }

    pub fn activation_point(&self) -> Option<EraId> {
        self.activation_point
    }

    pub fn new_validator_slots(&self) -> Option<u32> {
        self.new_validator_slots
    }

    pub fn new_auction_delay(&self) -> Option<u64> {
        self.new_auction_delay
    }

    pub fn new_locked_funds_period_millis(&self) -> Option<u64> {
        self.new_locked_funds_period_millis
    }

    pub fn new_round_seigniorage_rate(&self) -> Option<Ratio<u64>> {
        self.new_round_seigniorage_rate
    }

    pub fn new_unbonding_delay(&self) -> Option<u64> {
        self.new_unbonding_delay
    }

    pub fn global_state_update(&self) -> &BTreeMap<Key, StoredValue> {
        &self.global_state_update
    }

    pub fn with_pre_state_hash(&mut self, pre_state_hash: Blake2bHash) {
        self.pre_state_hash = pre_state_hash;
    }
}

#[derive(Clone, Error, Debug)]
pub enum ProtocolUpgradeError {
    #[error("Invalid upgrade config")]
    InvalidUpgradeConfig,
    #[error("Unable to retrieve system contract: {0}")]
    UnableToRetrieveSystemContract(String),
    #[error("Unable to retrieve system contract package: {0}")]
    UnableToRetrieveSystemContractPackage(String),
    #[error("Failed to disable previous version of system contract: {0}")]
    FailedToDisablePreviousVersion(String),
    #[error(transparent)]
    Bytesrepr(#[from] bytesrepr::Error),
}

pub(crate) struct SystemUpgrader<S>
where
    S: StateProvider,
{
    new_protocol_version: ProtocolVersion,
    protocol_data: ProtocolData,
    tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
}

impl<S> SystemUpgrader<S>
where
    S: StateProvider,
{
    pub(crate) fn new(
        new_protocol_version: ProtocolVersion,
        protocol_data: ProtocolData,
        tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
    ) -> Self {
        SystemUpgrader {
            new_protocol_version,
            protocol_data,
            tracking_copy,
        }
    }

    /// Bump major version for system contracts.
    pub(crate) fn upgrade_system_contracts_major_version(
        &self,
        correlation_id: CorrelationId,
    ) -> Result<(), ProtocolUpgradeError> {
        self.store_contract(
            correlation_id,
            self.protocol_data.mint(),
            MINT,
            mint::mint_entry_points(),
        )?;
        self.store_contract(
            correlation_id,
            self.protocol_data.auction(),
            AUCTION,
            auction::auction_entry_points(),
        )?;
        self.store_contract(
            correlation_id,
            self.protocol_data.handle_payment(),
            HANDLE_PAYMENT,
            handle_payment::handle_payment_entry_points(),
        )?;
        self.store_contract(
            correlation_id,
            self.protocol_data.standard_payment(),
            STANDARD_PAYMENT,
            standard_payment::standard_payment_entry_points(),
        )?;

        Ok(())
    }

    fn store_contract(
        &self,
        correlation_id: CorrelationId,
        contract_hash: ContractHash,
        contract_name: &str,
        entry_points: EntryPoints,
    ) -> Result<(), ProtocolUpgradeError> {
        let contract_key = Key::Hash(contract_hash.value());

        let mut contract = if let StoredValue::Contract(contract) = self
            .tracking_copy
            .borrow_mut()
            .read(correlation_id, &contract_key)
            .map_err(|_| {
                ProtocolUpgradeError::UnableToRetrieveSystemContract(contract_name.to_string())
            })?
            .ok_or_else(|| {
                ProtocolUpgradeError::UnableToRetrieveSystemContract(contract_name.to_string())
            })? {
            contract
        } else {
            return Err(ProtocolUpgradeError::UnableToRetrieveSystemContract(
                contract_name.to_string(),
            ));
        };

        let contract_package_key = Key::Hash(contract.contract_package_hash().value());

        let mut contract_package = if let StoredValue::ContractPackage(contract_package) = self
            .tracking_copy
            .borrow_mut()
            .read(correlation_id, &contract_package_key)
            .map_err(|_| {
                ProtocolUpgradeError::UnableToRetrieveSystemContractPackage(
                    contract_name.to_string(),
                )
            })?
            .ok_or_else(|| {
                ProtocolUpgradeError::UnableToRetrieveSystemContractPackage(
                    contract_name.to_string(),
                )
            })? {
            contract_package
        } else {
            return Err(ProtocolUpgradeError::UnableToRetrieveSystemContractPackage(
                contract_name.to_string(),
            ));
        };

        contract_package
            .disable_contract_version(contract_hash)
            .map_err(|_| {
                ProtocolUpgradeError::FailedToDisablePreviousVersion(contract_name.to_string())
            })?;
        contract.set_protocol_version(self.new_protocol_version);

        let new_contract = Contract::new(
            contract.contract_package_hash(),
            contract.contract_wasm_hash(),
            contract.named_keys().clone(),
            entry_points,
            self.new_protocol_version,
        );
        self.tracking_copy
            .borrow_mut()
            .write(contract_hash.into(), StoredValue::Contract(new_contract));

        contract_package
            .insert_contract_version(self.new_protocol_version.value().major, contract_hash);

        self.tracking_copy.borrow_mut().write(
            contract_package_key,
            StoredValue::ContractPackage(contract_package),
        );

        Ok(())
    }
}
