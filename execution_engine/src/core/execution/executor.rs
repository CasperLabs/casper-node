use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use parity_wasm::elements::Module;
use tracing::warn;
use wasmi::ModuleRef;

use casper_types::{
    account::AccountHash,
    bytesrepr::FromBytes,
    contracts::NamedKeys,
    system::{auction, handle_payment, mint, CallStackElement},
    BlockTime, CLTyped, CLValue, ContractPackage, DeployHash, EntryPoint, EntryPointType, Key,
    Phase, ProtocolVersion, RuntimeArgs,
};

use crate::{
    core::{
        engine_state::{
            execution_effect::ExecutionEffect, execution_result::ExecutionResult,
            system_contract_cache::SystemContractCache, EngineConfig,
        },
        execution::{address_generator::AddressGenerator, Error},
        runtime::{extract_access_rights_from_keys, instance_and_memory, Runtime},
        runtime_context::{self, RuntimeContext},
        tracking_copy::TrackingCopy,
    },
    shared::{account::Account, gas::Gas, newtypes::CorrelationId, stored_value::StoredValue},
    storage::{global_state::StateReader, protocol_data::ProtocolData},
};

macro_rules! on_fail_charge {
    ($fn:expr) => {
        match $fn {
            Ok(res) => res,
            Err(e) => {
                let exec_err: Error = e.into();
                warn!("Execution failed: {:?}", exec_err);
                return ExecutionResult::precondition_failure(exec_err.into());
            }
        }
    };
    ($fn:expr, $cost:expr, $transfers:expr) => {
        match $fn {
            Ok(res) => res,
            Err(e) => {
                let exec_err: Error = e.into();
                warn!("Execution failed: {:?}", exec_err);
                return ExecutionResult::Failure {
                    error: exec_err.into(),
                    execution_effect: Default::default(),
                    transfers: $transfers,
                    cost: $cost,
                };
            }
        }
    };
    ($fn:expr, $cost:expr, $execution_effect:expr, $transfers:expr) => {
        match $fn {
            Ok(res) => res,
            Err(e) => {
                let exec_err: Error = e.into();
                warn!("Execution failed: {:?}", exec_err);
                return ExecutionResult::Failure {
                    error: exec_err.into(),
                    execution_effect: $execution_effect,
                    transfers: $transfers,
                    cost: $cost,
                };
            }
        }
    };
}

pub struct Executor {
    config: EngineConfig,
}

#[allow(clippy::too_many_arguments)]
impl Executor {
    pub fn new(config: EngineConfig) -> Self {
        Executor { config }
    }

    pub fn config(&self) -> EngineConfig {
        self.config
    }

    pub fn exec<R>(
        &self,
        module: Module,
        entry_point: EntryPoint,
        args: RuntimeArgs,
        base_key: Key,
        account: &Account,
        named_keys: &mut NamedKeys,
        authorization_keys: BTreeSet<AccountHash>,
        blocktime: BlockTime,
        deploy_hash: DeployHash,
        gas_limit: Gas,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        phase: Phase,
        protocol_data: ProtocolData,
        system_contract_cache: SystemContractCache,
        contract_package: &ContractPackage,
        call_stack: Vec<CallStackElement>,
    ) -> ExecutionResult
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<Error>,
    {
        let entry_point_name = entry_point.name();
        let entry_point_type = entry_point.entry_point_type();
        let entry_point_access = entry_point.access();

        let (instance, memory) = on_fail_charge!(instance_and_memory(
            module.clone(),
            protocol_version,
            protocol_data.wasm_config()
        ));

        let access_rights = {
            let keys: Vec<Key> = named_keys.values().cloned().collect();
            extract_access_rights_from_keys(keys)
        };

        let hash_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let uref_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let target_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let gas_counter: Gas = Gas::default();
        let transfers = Vec::default();

        // Snapshot of effects before execution, so in case of error
        // only nonce update can be returned.
        let execution_effect = tracking_copy.borrow().effect();

        let context = RuntimeContext::new(
            tracking_copy,
            entry_point_type,
            named_keys,
            access_rights,
            args.clone(),
            authorization_keys,
            account,
            base_key,
            blocktime,
            deploy_hash,
            gas_limit,
            gas_counter,
            hash_address_generator,
            uref_address_generator,
            target_address_generator,
            protocol_version,
            correlation_id,
            phase,
            protocol_data,
            transfers,
        );

        let mut runtime = Runtime::new(
            self.config,
            system_contract_cache,
            memory,
            module,
            context,
            call_stack,
        );

        let accounts_access_rights = {
            let keys: Vec<Key> = account.named_keys().values().cloned().collect();
            extract_access_rights_from_keys(keys)
        };

        on_fail_charge!(runtime_context::validate_entry_point_access_with(
            contract_package,
            entry_point_access,
            |uref| runtime_context::uref_has_access_rights(uref, &accounts_access_rights)
        ));

        let call_stack = runtime.call_stack().to_owned();

        if runtime.is_mint(base_key) {
            match runtime.call_host_mint(
                protocol_version,
                entry_point.name(),
                &mut runtime.context().named_keys().to_owned(),
                &args,
                Default::default(),
                call_stack,
            ) {
                Ok(_value) => {
                    return ExecutionResult::Success {
                        execution_effect: runtime.context().effect(),
                        transfers: runtime.context().transfers().to_owned(),
                        cost: runtime.context().gas_counter(),
                    };
                }
                Err(error) => {
                    return ExecutionResult::Failure {
                        error: error.into(),
                        execution_effect,
                        transfers: runtime.context().transfers().to_owned(),
                        cost: runtime.context().gas_counter(),
                    };
                }
            }
        } else if runtime.is_handle_payment(base_key) {
            match runtime.call_host_handle_payment(
                protocol_version,
                entry_point.name(),
                &mut runtime.context().named_keys().to_owned(),
                &args,
                Default::default(),
                call_stack,
            ) {
                Ok(_value) => {
                    return ExecutionResult::Success {
                        execution_effect: runtime.context().effect(),
                        transfers: runtime.context().transfers().to_owned(),
                        cost: runtime.context().gas_counter(),
                    };
                }
                Err(error) => {
                    return ExecutionResult::Failure {
                        error: error.into(),
                        execution_effect,
                        transfers: runtime.context().transfers().to_owned(),
                        cost: runtime.context().gas_counter(),
                    };
                }
            }
        } else if runtime.is_auction(base_key) {
            match runtime.call_host_auction(
                protocol_version,
                entry_point.name(),
                &mut runtime.context().named_keys().to_owned(),
                &args,
                Default::default(),
                call_stack,
            ) {
                Ok(_value) => {
                    return ExecutionResult::Success {
                        execution_effect: runtime.context().effect(),
                        transfers: runtime.context().transfers().to_owned(),
                        cost: runtime.context().gas_counter(),
                    }
                }
                Err(error) => {
                    return ExecutionResult::Failure {
                        error: error.into(),
                        execution_effect,
                        transfers: runtime.context().transfers().to_owned(),
                        cost: runtime.context().gas_counter(),
                    }
                }
            }
        }
        on_fail_charge!(
            instance.invoke_export(entry_point_name, &[], &mut runtime),
            runtime.context().gas_counter(),
            execution_effect,
            runtime.context().transfers().to_owned()
        );

        ExecutionResult::Success {
            execution_effect: runtime.context().effect(),
            transfers: runtime.context().transfers().to_owned(),
            cost: runtime.context().gas_counter(),
        }
    }

    pub fn exec_standard_payment<R>(
        &self,
        system_module: Module,
        payment_args: RuntimeArgs,
        payment_base_key: Key,
        account: &Account,
        payment_named_keys: &mut NamedKeys,
        authorization_keys: BTreeSet<AccountHash>,
        blocktime: BlockTime,
        deploy_hash: DeployHash,
        payment_gas_limit: Gas,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        phase: Phase,
        protocol_data: ProtocolData,
        system_contract_cache: SystemContractCache,
        call_stack: Vec<CallStackElement>,
    ) -> ExecutionResult
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<Error>,
    {
        // use host side standard payment
        let hash_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let uref_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let transfer_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };

        let mut runtime = match self.create_runtime(
            system_module,
            EntryPointType::Session,
            payment_args,
            payment_named_keys,
            Default::default(),
            payment_base_key,
            account,
            authorization_keys,
            blocktime,
            deploy_hash,
            payment_gas_limit,
            hash_address_generator,
            uref_address_generator,
            transfer_address_generator,
            protocol_version,
            correlation_id,
            Rc::clone(&tracking_copy),
            phase,
            protocol_data,
            system_contract_cache,
            call_stack,
        ) {
            Ok((_instance, runtime)) => runtime,
            Err(error) => {
                return ExecutionResult::Failure {
                    error: error.into(),
                    execution_effect: Default::default(),
                    transfers: Vec::default(),
                    cost: Gas::default(),
                };
            }
        };

        let execution_effect = tracking_copy.borrow().effect();

        match runtime.call_host_standard_payment() {
            Ok(()) => ExecutionResult::Success {
                execution_effect: runtime.context().effect(),
                transfers: runtime.context().transfers().to_owned(),
                cost: runtime.context().gas_counter(),
            },
            Err(error) => ExecutionResult::Failure {
                error: error.into(),
                execution_effect,
                transfers: runtime.context().transfers().to_owned(),
                cost: runtime.context().gas_counter(),
            },
        }
    }

    pub fn exec_system_contract<R, T>(
        &self,
        direct_system_contract_call: DirectSystemContractCall,
        module: Module,
        runtime_args: RuntimeArgs,
        named_keys: &mut NamedKeys,
        extra_keys: &[Key],
        base_key: Key,
        account: &Account,
        authorization_keys: BTreeSet<AccountHash>,
        blocktime: BlockTime,
        deploy_hash: DeployHash,
        gas_limit: Gas,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        phase: Phase,
        protocol_data: ProtocolData,
        system_contract_cache: SystemContractCache,
        call_stack: Vec<CallStackElement>,
    ) -> (Option<T>, ExecutionResult)
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<Error>,
        T: FromBytes + CLTyped,
    {
        match direct_system_contract_call {
            DirectSystemContractCall::Slash
            | DirectSystemContractCall::RunAuction
            | DirectSystemContractCall::DistributeRewards => {
                if Some(protocol_data.auction().value()) != base_key.into_hash() {
                    panic!(
                        "{} should only be called with the auction contract",
                        direct_system_contract_call.entry_point_name()
                    );
                }
            }
            DirectSystemContractCall::FinalizePayment
            | DirectSystemContractCall::GetPaymentPurse => {
                if Some(protocol_data.handle_payment().value()) != base_key.into_hash() {
                    panic!(
                        "{} should only be called with the handle payment contract",
                        direct_system_contract_call.entry_point_name()
                    );
                }
            }
            DirectSystemContractCall::CreatePurse | DirectSystemContractCall::Transfer => {
                if Some(protocol_data.mint().value()) != base_key.into_hash() {
                    panic!(
                        "{} should only be called with the mint contract",
                        direct_system_contract_call.entry_point_name()
                    );
                }
            }
            DirectSystemContractCall::GetEraValidators => {
                if Some(protocol_data.auction().value()) != base_key.into_hash() {
                    panic!(
                        "{} should only be called with the auction contract",
                        direct_system_contract_call.entry_point_name()
                    );
                }
            }
        }

        let hash_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let uref_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let transfer_address_generator = {
            let generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
            Rc::new(RefCell::new(generator))
        };
        let gas_counter = Gas::default(); // maybe const?

        // Snapshot of effects before execution, so in case of error only nonce update
        // can be returned.
        let execution_effect = tracking_copy.borrow().effect();

        let transfers = Vec::default();

        let (_, runtime) = match self.create_runtime(
            module,
            EntryPointType::Contract,
            runtime_args.clone(),
            named_keys,
            extra_keys,
            base_key,
            account,
            authorization_keys,
            blocktime,
            deploy_hash,
            gas_limit,
            hash_address_generator,
            uref_address_generator,
            transfer_address_generator,
            protocol_version,
            correlation_id,
            tracking_copy,
            phase,
            protocol_data,
            system_contract_cache,
            call_stack,
        ) {
            Ok((instance, runtime)) => (instance, runtime),
            Err(error) => {
                return ExecutionResult::Failure {
                    execution_effect,
                    transfers,
                    cost: gas_counter,
                    error: error.into(),
                }
                .take_without_ret()
            }
        };

        let mut inner_named_keys = runtime.context().named_keys().clone();
        let ret = direct_system_contract_call.host_exec(
            runtime,
            protocol_version,
            &mut inner_named_keys,
            &runtime_args,
            extra_keys,
            execution_effect,
        );
        *named_keys = inner_named_keys;
        ret
    }

    /// Used to execute arbitrary wasm; necessary for running system contract installers / upgraders
    /// This is not meant to be used for executing system contracts.
    pub fn exec_wasm_direct<R, T>(
        &self,
        module: Module,
        entry_point_name: &str,
        args: RuntimeArgs,
        account: &mut Account,
        authorization_keys: BTreeSet<AccountHash>,
        blocktime: BlockTime,
        deploy_hash: DeployHash,
        gas_limit: Gas,
        hash_address_generator: Rc<RefCell<AddressGenerator>>,
        uref_address_generator: Rc<RefCell<AddressGenerator>>,
        transfer_address_generator: Rc<RefCell<AddressGenerator>>,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        phase: Phase,
        protocol_data: ProtocolData,
        system_contract_cache: SystemContractCache,
        call_stack: Vec<CallStackElement>,
    ) -> Result<T, Error>
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<Error>,
        T: FromBytes + CLTyped,
    {
        let mut named_keys: NamedKeys = account.named_keys().clone();
        let base_key = account.account_hash().into();

        let (instance, mut runtime) = self.create_runtime(
            module,
            EntryPointType::Session,
            args,
            &mut named_keys,
            Default::default(),
            base_key,
            account,
            authorization_keys,
            blocktime,
            deploy_hash,
            gas_limit,
            hash_address_generator,
            uref_address_generator,
            transfer_address_generator,
            protocol_version,
            correlation_id,
            tracking_copy,
            phase,
            protocol_data,
            system_contract_cache,
            call_stack,
        )?;

        let error: wasmi::Error = match instance.invoke_export(entry_point_name, &[], &mut runtime)
        {
            Err(error) => error,
            Ok(_) => {
                // This duplicates the behavior of runtime sub_call.
                // If `instance.invoke_export` returns `Ok` and the `host_buffer` is `None`, the
                // contract's execution succeeded but did not explicitly call `runtime::ret()`.
                // Treat as though the execution returned the unit type `()` as per Rust
                // functions which don't specify a return value.
                let result = runtime.take_host_buffer().unwrap_or(CLValue::from_t(())?);
                let ret = result.into_t()?;
                *account.named_keys_mut() = named_keys;
                return Ok(ret);
            }
        };

        let return_value: CLValue = match error
            .as_host_error()
            .and_then(|host_error| host_error.downcast_ref::<Error>())
        {
            Some(Error::Ret(_)) => runtime
                .take_host_buffer()
                .ok_or(Error::ExpectedReturnValue)?,
            Some(Error::Revert(code)) => return Err(Error::Revert(*code)),
            Some(error) => return Err(error.clone()),
            _ => return Err(Error::Interpreter(error.into())),
        };

        let ret = return_value.into_t()?;
        *account.named_keys_mut() = named_keys;
        Ok(ret)
    }

    pub fn create_runtime<'a, R>(
        &self,
        module: Module,
        entry_point_type: EntryPointType,
        runtime_args: RuntimeArgs,
        named_keys: &'a mut NamedKeys,
        extra_keys: &[Key],
        base_key: Key,
        account: &'a Account,
        authorization_keys: BTreeSet<AccountHash>,
        blocktime: BlockTime,
        deploy_hash: DeployHash,
        gas_limit: Gas,
        hash_address_generator: Rc<RefCell<AddressGenerator>>,
        uref_address_generator: Rc<RefCell<AddressGenerator>>,
        transfer_address_generator: Rc<RefCell<AddressGenerator>>,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        phase: Phase,
        protocol_data: ProtocolData,
        system_contract_cache: SystemContractCache,
        call_stack: Vec<CallStackElement>,
    ) -> Result<(ModuleRef, Runtime<'a, R>), Error>
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<Error>,
    {
        let access_rights = {
            let mut keys: Vec<Key> = named_keys.values().cloned().collect();
            keys.extend(extra_keys);
            extract_access_rights_from_keys(keys)
        };

        let gas_counter = Gas::default();
        let transfers = Vec::default();

        let runtime_context = RuntimeContext::new(
            tracking_copy,
            entry_point_type,
            named_keys,
            access_rights,
            runtime_args,
            authorization_keys,
            account,
            base_key,
            blocktime,
            deploy_hash,
            gas_limit,
            gas_counter,
            hash_address_generator,
            uref_address_generator,
            transfer_address_generator,
            protocol_version,
            correlation_id,
            phase,
            protocol_data,
            transfers,
        );

        let (instance, memory) = instance_and_memory(
            module.clone(),
            protocol_version,
            protocol_data.wasm_config(),
        )?;

        let runtime = Runtime::new(
            self.config,
            system_contract_cache,
            memory,
            module,
            runtime_context,
            call_stack,
        );

        Ok((instance, runtime))
    }
}

pub enum DirectSystemContractCall {
    Slash,
    RunAuction,
    DistributeRewards,
    FinalizePayment,
    CreatePurse,
    Transfer,
    GetEraValidators,
    GetPaymentPurse,
}

impl DirectSystemContractCall {
    fn entry_point_name(&self) -> &str {
        match self {
            DirectSystemContractCall::Slash => auction::METHOD_SLASH,
            DirectSystemContractCall::RunAuction => auction::METHOD_RUN_AUCTION,
            DirectSystemContractCall::DistributeRewards => auction::METHOD_DISTRIBUTE,
            DirectSystemContractCall::FinalizePayment => handle_payment::METHOD_FINALIZE_PAYMENT,
            DirectSystemContractCall::CreatePurse => mint::METHOD_CREATE,
            DirectSystemContractCall::Transfer => mint::METHOD_TRANSFER,
            DirectSystemContractCall::GetEraValidators => auction::METHOD_GET_ERA_VALIDATORS,
            DirectSystemContractCall::GetPaymentPurse => handle_payment::METHOD_GET_PAYMENT_PURSE,
        }
    }

    fn host_exec<R, T>(
        &self,
        mut runtime: Runtime<R>,
        protocol_version: ProtocolVersion,
        named_keys: &mut NamedKeys,
        runtime_args: &RuntimeArgs,
        extra_keys: &[Key],
        execution_effect: ExecutionEffect,
    ) -> (Option<T>, ExecutionResult)
    where
        R: StateReader<Key, StoredValue>,
        R::Error: Into<Error>,
        T: FromBytes + CLTyped,
    {
        let entry_point_name = self.entry_point_name();

        let call_stack = runtime.call_stack().to_owned();

        let result = match self {
            DirectSystemContractCall::Slash
            | DirectSystemContractCall::RunAuction
            | DirectSystemContractCall::DistributeRewards => runtime.call_host_auction(
                protocol_version,
                entry_point_name,
                named_keys,
                runtime_args,
                extra_keys,
                call_stack,
            ),
            DirectSystemContractCall::FinalizePayment => runtime.call_host_handle_payment(
                protocol_version,
                entry_point_name,
                named_keys,
                runtime_args,
                extra_keys,
                call_stack,
            ),
            DirectSystemContractCall::CreatePurse | DirectSystemContractCall::Transfer => runtime
                .call_host_mint(
                    protocol_version,
                    entry_point_name,
                    named_keys,
                    runtime_args,
                    extra_keys,
                    call_stack,
                ),
            DirectSystemContractCall::GetEraValidators => runtime.call_host_auction(
                protocol_version,
                entry_point_name,
                named_keys,
                runtime_args,
                extra_keys,
                call_stack,
            ),

            DirectSystemContractCall::GetPaymentPurse => runtime.call_host_handle_payment(
                protocol_version,
                entry_point_name,
                named_keys,
                runtime_args,
                extra_keys,
                call_stack,
            ),
        };

        match result {
            Ok(value) => match value.into_t() {
                Ok(ret) => ExecutionResult::Success {
                    execution_effect: runtime.context().effect(),
                    transfers: runtime.context().transfers().to_owned(),
                    cost: runtime.context().gas_counter(),
                }
                .take_with_ret(ret),
                Err(error) => ExecutionResult::Failure {
                    error: Error::CLValue(error).into(),
                    execution_effect,
                    transfers: runtime.context().transfers().to_owned(),
                    cost: runtime.context().gas_counter(),
                }
                .take_without_ret(),
            },
            Err(error) => ExecutionResult::Failure {
                error: error.into(),
                execution_effect,
                transfers: runtime.context().transfers().to_owned(),
                cost: runtime.context().gas_counter(),
            }
            .take_without_ret(),
        }
    }
}
