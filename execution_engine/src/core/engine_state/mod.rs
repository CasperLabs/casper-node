pub mod balance;
pub mod deploy_item;
pub mod engine_config;
pub mod era_validators;
mod error;
pub mod executable_deploy_item;
pub mod execute_request;
pub mod execution_effect;
pub mod execution_result;
pub mod genesis;
pub mod op;
pub mod query;
pub mod run_genesis_request;
pub mod step;
pub mod system_contract_cache;
mod transfer;
pub mod upgrade;

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    convert::TryFrom,
    iter::FromIterator,
    rc::Rc,
};

use num_rational::Ratio;
use once_cell::sync::Lazy;
use tracing::{debug, error};

use casper_types::{
    account::AccountHash,
    bytesrepr::ToBytes,
    contracts::NamedKeys,
    system::{
        auction::{
            EraValidators, ARG_ERA_END_TIMESTAMP_MILLIS, ARG_EVICTED_VALIDATORS,
            ARG_REWARD_FACTORS, ARG_VALIDATOR_PUBLIC_KEYS, AUCTION_DELAY_KEY,
            LOCKED_FUNDS_PERIOD_KEY, UNBONDING_DELAY_KEY, VALIDATOR_SLOTS_KEY,
        },
        handle_payment,
        mint::{self, ROUND_SEIGNIORAGE_RATE_KEY},
    },
    AccessRights, ApiError, BlockTime, CLValue, Contract, DeployHash, DeployInfo, Key, KeyTag,
    Phase, ProtocolVersion, PublicKey, RuntimeArgs, URef, U512,
};

pub use self::{
    balance::{BalanceRequest, BalanceResult},
    deploy_item::DeployItem,
    engine_config::EngineConfig,
    era_validators::{GetEraValidatorsError, GetEraValidatorsRequest},
    error::Error,
    executable_deploy_item::ExecutableDeployItem,
    execute_request::ExecuteRequest,
    execution::Error as ExecError,
    execution_result::{ExecutionResult, ExecutionResults, ForcedTransferResult},
    genesis::{ExecConfig, GenesisAccount, GenesisResult},
    query::{GetBidsRequest, GetBidsResult, QueryRequest, QueryResult},
    step::{RewardItem, SlashItem, StepRequest, StepResult},
    system_contract_cache::SystemContractCache,
    transfer::{TransferArgs, TransferRuntimeArgsBuilder, TransferTargetMode},
    upgrade::{UpgradeConfig, UpgradeResult},
};
use crate::{
    core::{
        engine_state::{
            executable_deploy_item::DeployMetadata, execution_result::ExecutionResultBuilder,
            genesis::GenesisInstaller, upgrade::SystemUpgrader,
        },
        execution::{self, DirectSystemContractCall, Executor},
        tracking_copy::{TrackingCopy, TrackingCopyExt},
    },
    shared::{
        account::Account,
        additive_map::AdditiveMap,
        gas::Gas,
        motes::Motes,
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
        transform::Transform,
        wasm_prep::Preprocessor,
    },
    storage::{
        global_state::{CommitResult, StateProvider},
        protocol_data::ProtocolData,
        trie::Trie,
    },
};

pub const MAX_PAYMENT_AMOUNT: u64 = 2_500_000_000;
pub static MAX_PAYMENT: Lazy<U512> = Lazy::new(|| U512::from(MAX_PAYMENT_AMOUNT));

/// Gas/motes conversion rate of wasmless transfer cost is always 1 regardless of what user wants to
/// pay.
pub const WASMLESS_TRANSFER_FIXED_GAS_PRICE: u64 = 1;

#[derive(Debug)]
pub struct EngineState<S> {
    config: EngineConfig,
    system_contract_cache: SystemContractCache,
    state: S,
}

impl<S> EngineState<S>
where
    S: StateProvider,
    S::Error: Into<execution::Error>,
{
    pub fn new(state: S, config: EngineConfig) -> EngineState<S> {
        let system_contract_cache = Default::default();
        EngineState {
            config,
            system_contract_cache,
            state,
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn state(&self) -> &S {
        &self.state
    }

    pub fn get_protocol_data(
        &self,
        protocol_version: ProtocolVersion,
    ) -> Result<Option<ProtocolData>, Error> {
        match self.state.get_protocol_data(protocol_version) {
            Ok(Some(protocol_data)) => Ok(Some(protocol_data)),
            Err(error) => Err(Error::Exec(error.into())),
            _ => Ok(None),
        }
    }

    pub fn commit_genesis(
        &self,
        correlation_id: CorrelationId,
        genesis_config_hash: Blake2bHash,
        protocol_version: ProtocolVersion,
        ee_config: &ExecConfig,
    ) -> Result<GenesisResult, Error> {
        // Preliminaries
        let initial_root_hash = self.state.empty_root();
        let system_config = ee_config.system_config();

        let tracking_copy = match self.tracking_copy(initial_root_hash) {
            Ok(Some(tracking_copy)) => Rc::new(RefCell::new(tracking_copy)),
            // NOTE: As genesis is ran once per instance condition below is considered programming
            // error
            Ok(None) => panic!("state has not been initialized properly"),
            Err(error) => return Err(error),
        };

        let wasm_config = ee_config.wasm_config();
        let preprocessor = Preprocessor::new(*wasm_config);

        let system_module = tracking_copy
            .borrow_mut()
            .get_system_module(&preprocessor)?;

        let mut genesis_installer: GenesisInstaller<S> = GenesisInstaller::new(
            genesis_config_hash,
            protocol_version,
            correlation_id,
            self.config,
            ee_config.clone(),
            tracking_copy,
            system_module,
        );

        // Create mint
        let mint_hash = genesis_installer.create_mint()?;

        // Create accounts
        genesis_installer.create_accounts()?;

        // Create handle payment
        let handle_payment_hash = genesis_installer.create_handle_payment()?;

        // Create auction
        let auction_hash = genesis_installer.create_auction()?;

        // Create standard payment
        let standard_payment_hash = genesis_installer.create_standard_payment();

        // Associate given CostTable with given ProtocolVersion.
        {
            let protocol_data = ProtocolData::new(
                *wasm_config,
                *system_config,
                mint_hash,
                handle_payment_hash,
                standard_payment_hash,
                auction_hash,
            );

            self.state
                .put_protocol_data(protocol_version, &protocol_data)
                .map_err(Into::into)?;
        }

        // Commit the transforms.
        let execution_effect = genesis_installer.finalize();

        let commit_result = self
            .state
            .commit(
                correlation_id,
                initial_root_hash,
                execution_effect.transforms.to_owned(),
            )
            .map_err(Into::into)?;

        // Return the result
        let genesis_result = GenesisResult::from_commit_result(commit_result, execution_effect);

        Ok(genesis_result)
    }

    pub fn commit_upgrade(
        &self,
        correlation_id: CorrelationId,
        upgrade_config: UpgradeConfig,
    ) -> Result<UpgradeResult, Error> {
        // per specification:
        // https://casperlabs.atlassian.net/wiki/spaces/EN/pages/139854367/Upgrading+System+Contracts+Specification

        // 3.1.1.1.1.1 validate pre state hash exists
        // 3.1.2.1 get a tracking_copy at the provided pre_state_hash
        let pre_state_hash = upgrade_config.pre_state_hash();
        let tracking_copy = match self.tracking_copy(pre_state_hash)? {
            Some(tracking_copy) => Rc::new(RefCell::new(tracking_copy)),
            None => return Ok(UpgradeResult::RootNotFound),
        };

        // 3.1.1.1.1.2 current protocol version is required
        let current_protocol_version = upgrade_config.current_protocol_version();
        let current_protocol_data = match self.state.get_protocol_data(current_protocol_version) {
            Ok(Some(protocol_data)) => protocol_data,
            Ok(None) => {
                return Err(Error::InvalidProtocolVersion(current_protocol_version));
            }
            Err(error) => {
                return Err(Error::Exec(error.into()));
            }
        };

        // 3.1.1.1.1.3 activation point is not currently used by EE; skipping
        // 3.1.1.1.1.4 upgrade point protocol version validation
        let new_protocol_version = upgrade_config.new_protocol_version();

        let upgrade_check_result =
            current_protocol_version.check_next_version(&new_protocol_version);

        if upgrade_check_result.is_invalid() {
            return Err(Error::InvalidProtocolVersion(new_protocol_version));
        }

        // 3.1.1.1.1.5 bump system contract major versions
        if upgrade_check_result.is_major_version() {
            let system_upgrader: SystemUpgrader<S> = SystemUpgrader::new(
                new_protocol_version,
                current_protocol_data,
                tracking_copy.clone(),
            );

            system_upgrader
                .upgrade_system_contracts_major_version(correlation_id)
                .map_err(Error::ProtocolUpgrade)?;
        }

        // 3.1.1.1.1.6 resolve wasm CostTable for new protocol version
        let new_wasm_config = match upgrade_config.wasm_config() {
            Some(new_wasm_costs) => new_wasm_costs,
            None => current_protocol_data.wasm_config(),
        };

        let new_system_config = match upgrade_config.system_config() {
            Some(new_system_config) => new_system_config,
            None => current_protocol_data.system_config(),
        };

        // 3.1.2.2 persist wasm CostTable
        let new_protocol_data = ProtocolData::new(
            *new_wasm_config,
            *new_system_config,
            current_protocol_data.mint(),
            current_protocol_data.handle_payment(),
            current_protocol_data.standard_payment(),
            current_protocol_data.auction(),
        );

        self.state
            .put_protocol_data(new_protocol_version, &new_protocol_data)
            .map_err(Into::into)?;

        // 3.1.1.1.1.7 new total validator slots is optional
        if let Some(new_validator_slots) = upgrade_config.new_validator_slots() {
            // 3.1.2.4 if new total validator slots is provided, update auction contract state
            let auction_contract = tracking_copy
                .borrow_mut()
                .get_contract(correlation_id, new_protocol_data.auction())?;

            let validator_slots_key = auction_contract.named_keys()[VALIDATOR_SLOTS_KEY];
            let value = StoredValue::CLValue(
                CLValue::from_t(new_validator_slots)
                    .map_err(|_| Error::Bytesrepr("new_validator_slots".to_string()))?,
            );
            tracking_copy.borrow_mut().write(validator_slots_key, value);
        }

        if let Some(new_auction_delay) = upgrade_config.new_auction_delay() {
            let auction_contract = tracking_copy
                .borrow_mut()
                .get_contract(correlation_id, new_protocol_data.auction())?;

            let auction_delay_key = auction_contract.named_keys()[AUCTION_DELAY_KEY];
            let value = StoredValue::CLValue(
                CLValue::from_t(new_auction_delay)
                    .map_err(|_| Error::Bytesrepr("new_auction_delay".to_string()))?,
            );
            tracking_copy.borrow_mut().write(auction_delay_key, value);
        }

        if let Some(new_locked_funds_period) = upgrade_config.new_locked_funds_period_millis() {
            let auction_contract = tracking_copy
                .borrow_mut()
                .get_contract(correlation_id, new_protocol_data.auction())?;

            let locked_funds_period_key = auction_contract.named_keys()[LOCKED_FUNDS_PERIOD_KEY];
            let value = StoredValue::CLValue(
                CLValue::from_t(new_locked_funds_period)
                    .map_err(|_| Error::Bytesrepr("new_locked_funds_period".to_string()))?,
            );
            tracking_copy
                .borrow_mut()
                .write(locked_funds_period_key, value);
        }

        if let Some(new_unbonding_delay) = upgrade_config.new_unbonding_delay() {
            let auction_contract = tracking_copy
                .borrow_mut()
                .get_contract(correlation_id, new_protocol_data.auction())?;

            let unbonding_delay_key = auction_contract.named_keys()[UNBONDING_DELAY_KEY];
            let value = StoredValue::CLValue(
                CLValue::from_t(new_unbonding_delay)
                    .map_err(|_| Error::Bytesrepr("new_unbonding_delay".to_string()))?,
            );
            tracking_copy.borrow_mut().write(unbonding_delay_key, value);
        }

        if let Some(new_round_seigniorage_rate) = upgrade_config.new_round_seigniorage_rate() {
            let new_round_seigniorage_rate: Ratio<U512> = {
                let (numer, denom) = new_round_seigniorage_rate.into();
                Ratio::new(numer.into(), denom.into())
            };

            let mint_contract = tracking_copy
                .borrow_mut()
                .get_contract(correlation_id, new_protocol_data.mint())?;

            let locked_funds_period_key = mint_contract.named_keys()[ROUND_SEIGNIORAGE_RATE_KEY];
            let value = StoredValue::CLValue(
                CLValue::from_t(new_round_seigniorage_rate)
                    .map_err(|_| Error::Bytesrepr("new_round_seigniorage_rate".to_string()))?,
            );
            tracking_copy
                .borrow_mut()
                .write(locked_funds_period_key, value);
        }

        // apply the arbitrary modifications
        for (key, value) in upgrade_config.global_state_update() {
            tracking_copy.borrow_mut().write(*key, value.clone());
        }

        let effects = tracking_copy.borrow().effect();

        // commit
        let commit_result = self
            .state
            .commit(
                correlation_id,
                pre_state_hash,
                effects.transforms.to_owned(),
            )
            .map_err(Into::into)?;

        // return result and effects
        Ok(UpgradeResult::from_commit_result(commit_result, effects))
    }

    pub fn tracking_copy(
        &self,
        hash: Blake2bHash,
    ) -> Result<Option<TrackingCopy<S::Reader>>, Error> {
        match self.state.checkout(hash).map_err(Into::into)? {
            Some(tc) => Ok(Some(TrackingCopy::new(tc))),
            None => Ok(None),
        }
    }

    pub fn run_query(
        &self,
        correlation_id: CorrelationId,
        query_request: QueryRequest,
    ) -> Result<QueryResult, Error> {
        let tracking_copy = match self.tracking_copy(query_request.state_hash())? {
            Some(tracking_copy) => Rc::new(RefCell::new(tracking_copy)),
            None => return Ok(QueryResult::RootNotFound),
        };

        let tracking_copy = tracking_copy.borrow();

        Ok(tracking_copy
            .query(
                correlation_id,
                &self.config,
                query_request.key(),
                query_request.path(),
            )
            .map_err(|err| Error::Exec(err.into()))?
            .into())
    }

    pub fn run_execute(
        &self,
        correlation_id: CorrelationId,
        mut exec_request: ExecuteRequest,
    ) -> Result<ExecutionResults, Error> {
        let executor = Executor::new(self.config);

        let deploys = exec_request.take_deploys();
        let mut results = ExecutionResults::with_capacity(deploys.len());

        for deploy_item in deploys {
            let result = match deploy_item.session {
                ExecutableDeployItem::Transfer { .. } => self.transfer(
                    correlation_id,
                    &executor,
                    exec_request.protocol_version,
                    exec_request.parent_state_hash,
                    BlockTime::new(exec_request.block_time),
                    deploy_item,
                    exec_request.proposer.clone(),
                ),
                _ => self.deploy(
                    correlation_id,
                    &executor,
                    exec_request.protocol_version,
                    exec_request.parent_state_hash,
                    BlockTime::new(exec_request.block_time),
                    deploy_item,
                    exec_request.proposer.clone(),
                ),
            };
            match result {
                Ok(result) => results.push_back(result),
                Err(error) => {
                    return Err(error);
                }
            };
        }

        Ok(results)
    }

    fn get_authorized_account(
        &self,
        correlation_id: CorrelationId,
        account_hash: AccountHash,
        authorization_keys: &BTreeSet<AccountHash>,
        tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
    ) -> Result<Account, Error> {
        let account: Account = match tracking_copy
            .borrow_mut()
            .get_account(correlation_id, account_hash)
        {
            Ok(account) => account,
            Err(_) => {
                return Err(error::Error::Authorization);
            }
        };

        // Authorize using provided authorization keys
        if !account.can_authorize(authorization_keys) {
            return Err(error::Error::Authorization);
        }

        // Check total key weight against deploy threshold
        if !account.can_deploy_with(authorization_keys) {
            return Err(execution::Error::DeploymentAuthorizationFailure.into());
        }

        Ok(account)
    }

    pub fn get_purse_balance(
        &self,
        correlation_id: CorrelationId,
        state_hash: Blake2bHash,
        purse_uref: URef,
    ) -> Result<BalanceResult, Error> {
        let tracking_copy = match self.tracking_copy(state_hash)? {
            Some(tracking_copy) => tracking_copy,
            None => return Ok(BalanceResult::RootNotFound),
        };
        let purse_balance_key =
            tracking_copy.get_purse_balance_key(correlation_id, purse_uref.into())?;
        let (balance, proof) =
            tracking_copy.get_purse_balance_with_proof(correlation_id, purse_balance_key)?;
        let proof = Box::new(proof);
        let motes = balance.value();
        Ok(BalanceResult::Success { motes, proof })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn transfer(
        &self,
        correlation_id: CorrelationId,
        executor: &Executor,
        protocol_version: ProtocolVersion,
        prestate_hash: Blake2bHash,
        blocktime: BlockTime,
        deploy_item: DeployItem,
        proposer: PublicKey,
    ) -> Result<ExecutionResult, Error> {
        let protocol_data = match self.state.get_protocol_data(protocol_version) {
            Ok(Some(protocol_data)) => protocol_data,
            Ok(None) => {
                let error = Error::InvalidProtocolVersion(protocol_version);
                return Ok(ExecutionResult::precondition_failure(error));
            }
            Err(error) => {
                return Ok(ExecutionResult::precondition_failure(Error::Exec(
                    error.into(),
                )));
            }
        };

        let tracking_copy = match self.tracking_copy(prestate_hash) {
            Err(error) => return Ok(ExecutionResult::precondition_failure(error)),
            Ok(None) => return Err(Error::RootNotFound(prestate_hash)),
            Ok(Some(tracking_copy)) => Rc::new(RefCell::new(tracking_copy)),
        };

        let preprocessor = {
            let wasm_config = protocol_data.wasm_config();
            Preprocessor::new(*wasm_config)
        };

        let system_module = {
            match tracking_copy.borrow_mut().get_system_module(&preprocessor) {
                Ok(module) => module,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            }
        };

        let base_key = Key::Account(deploy_item.address);

        let account_public_key = match base_key.into_account() {
            Some(account_addr) => account_addr,
            None => {
                return Ok(ExecutionResult::precondition_failure(
                    error::Error::Authorization,
                ));
            }
        };

        let authorization_keys = deploy_item.authorization_keys;

        let account = match self.get_authorized_account(
            correlation_id,
            account_public_key,
            &authorization_keys,
            Rc::clone(&tracking_copy),
        ) {
            Ok(account) => account,
            Err(e) => return Ok(ExecutionResult::precondition_failure(e)),
        };

        let proposer_addr = proposer.to_account_hash();
        let proposer_account = match tracking_copy
            .borrow_mut()
            .get_account(correlation_id, proposer_addr)
        {
            Ok(proposer) => proposer,
            Err(error) => return Ok(ExecutionResult::precondition_failure(Error::Exec(error))),
        };

        let mint_contract = match tracking_copy
            .borrow_mut()
            .get_contract(correlation_id, protocol_data.mint())
        {
            Ok(contract) => contract,
            Err(error) => {
                return Ok(ExecutionResult::precondition_failure(error.into()));
            }
        };

        let mut mint_named_keys = mint_contract.named_keys().to_owned();
        let mut mint_extra_keys: Vec<Key> = vec![];
        let mint_base_key = Key::from(protocol_data.mint());

        let handle_payment_contract = match tracking_copy
            .borrow_mut()
            .get_contract(correlation_id, protocol_data.handle_payment())
        {
            Ok(contract) => contract,
            Err(error) => {
                return Ok(ExecutionResult::precondition_failure(error.into()));
            }
        };

        let mut handle_payment_named_keys = handle_payment_contract.named_keys().to_owned();
        let handle_payment_extra_keys: Vec<Key> = vec![];
        let handle_payment_base_key = Key::from(protocol_data.handle_payment());

        let gas_limit = Gas::new(U512::from(std::u64::MAX));

        let wasmless_transfer_gas_cost = Gas::new(U512::from(
            protocol_data.system_config().wasmless_transfer_cost(),
        ));

        let wasmless_transfer_motes = match Motes::from_gas(
            wasmless_transfer_gas_cost,
            WASMLESS_TRANSFER_FIXED_GAS_PRICE,
        ) {
            Some(motes) => motes,
            None => {
                return Ok(ExecutionResult::precondition_failure(
                    Error::GasConversionOverflow,
                ))
            }
        };

        let proposer_main_purse_balance_key = {
            let proposer_main_purse = proposer_account.main_purse();

            match tracking_copy
                .borrow_mut()
                .get_purse_balance_key(correlation_id, proposer_main_purse.into())
            {
                Ok(balance_key) => balance_key,
                Err(error) => return Ok(ExecutionResult::precondition_failure(Error::Exec(error))),
            }
        };

        let proposer_purse = proposer_account.main_purse();

        let account_main_purse = account.main_purse();

        let account_main_purse_balance_key = match tracking_copy
            .borrow_mut()
            .get_purse_balance_key(correlation_id, account_main_purse.into())
        {
            Ok(balance_key) => balance_key,
            Err(error) => return Ok(ExecutionResult::precondition_failure(Error::Exec(error))),
        };

        let account_main_purse_balance = match tracking_copy
            .borrow_mut()
            .get_purse_balance(correlation_id, account_main_purse_balance_key)
        {
            Ok(balance_key) => balance_key,
            Err(error) => return Ok(ExecutionResult::precondition_failure(Error::Exec(error))),
        };

        if account_main_purse_balance < wasmless_transfer_motes {
            // We don't have minimum balance to operate and therefore we can't charge for user
            // errors.
            return Ok(ExecutionResult::precondition_failure(
                Error::InsufficientPayment,
            ));
        }

        // Function below creates an ExecutionResult with precomputed effects of "finalize_payment".
        let make_charged_execution_failure = |error| match ExecutionResult::new_payment_code_error(
            error,
            wasmless_transfer_motes,
            account_main_purse_balance,
            wasmless_transfer_gas_cost,
            account_main_purse_balance_key,
            proposer_main_purse_balance_key,
        ) {
            Ok(execution_result) => execution_result,
            Err(error) => {
                let exec_error = ExecError::from(error);
                ExecutionResult::precondition_failure(exec_error.into())
            }
        };

        // All wasmless transfer preconditions are met.
        // Any error that occurs in logic below this point would result in a charge for user error.

        let mut runtime_args_builder =
            TransferRuntimeArgsBuilder::new(deploy_item.session.args().clone());

        match runtime_args_builder.transfer_target_mode(correlation_id, Rc::clone(&tracking_copy)) {
            Ok(mode) => match mode {
                TransferTargetMode::Unknown | TransferTargetMode::PurseExists(_) => { /* noop */ }
                TransferTargetMode::CreateAccount(public_key) => {
                    let (maybe_uref, execution_result): (Option<URef>, ExecutionResult) = executor
                        .exec_system_contract(
                            DirectSystemContractCall::CreatePurse,
                            system_module.clone(),
                            RuntimeArgs::new(), // mint create takes no arguments
                            &mut mint_named_keys,
                            Default::default(),
                            mint_base_key,
                            &account,
                            authorization_keys.clone(),
                            blocktime,
                            deploy_item.deploy_hash,
                            gas_limit,
                            protocol_version,
                            correlation_id,
                            Rc::clone(&tracking_copy),
                            Phase::Session,
                            protocol_data,
                            SystemContractCache::clone(&self.system_contract_cache),
                        );
                    match maybe_uref {
                        Some(main_purse) => {
                            let new_account =
                                Account::create(public_key, Default::default(), main_purse);
                            mint_extra_keys.push(Key::from(main_purse));
                            // write new account
                            tracking_copy
                                .borrow_mut()
                                .write(Key::Account(public_key), StoredValue::Account(new_account))
                        }
                        None => {
                            // This case implies that the execution_result is a failure variant as
                            // implemented inside host_exec().
                            let error = execution_result
                                .take_error()
                                .unwrap_or(Error::InsufficientPayment);
                            return Ok(make_charged_execution_failure(error));
                        }
                    }
                }
            },
            Err(error) => return Ok(make_charged_execution_failure(error)),
        }

        let transfer_args =
            match runtime_args_builder.build(&account, correlation_id, Rc::clone(&tracking_copy)) {
                Ok(transfer_args) => transfer_args,
                Err(error) => return Ok(make_charged_execution_failure(error)),
            };

        // Construct a payment code that will put cost of wasmless payment into payment purse
        let payment_result = {
            // Check source purses minimum balance
            let source_uref = transfer_args.source();
            let source_purse_balance = if source_uref != account_main_purse {
                let source_purse_balance_key = match tracking_copy
                    .borrow_mut()
                    .get_purse_balance_key(correlation_id, Key::URef(source_uref))
                {
                    Ok(purse_balance_key) => purse_balance_key,
                    Err(error) => return Ok(make_charged_execution_failure(Error::Exec(error))),
                };

                match tracking_copy
                    .borrow_mut()
                    .get_purse_balance(correlation_id, source_purse_balance_key)
                {
                    Ok(purse_balance) => purse_balance,
                    Err(error) => return Ok(make_charged_execution_failure(Error::Exec(error))),
                }
            } else {
                // If source purse is main purse then we already have the balance.
                account_main_purse_balance
            };

            let transfer_amount_motes = Motes::new(transfer_args.amount());

            match wasmless_transfer_motes.checked_add(transfer_amount_motes) {
                Some(total_amount) if source_purse_balance < total_amount => {
                    // We can't continue if the minimum funds in source purse are lower than the
                    // required cost.
                    return Ok(make_charged_execution_failure(Error::InsufficientPayment));
                }
                None => {
                    // When trying to send too much that could cause an overflow.
                    return Ok(make_charged_execution_failure(Error::InsufficientPayment));
                }
                Some(_) => {}
            }

            let (payment_uref, get_payment_purse_result): (Option<URef>, ExecutionResult) =
                executor.exec_system_contract(
                    DirectSystemContractCall::GetPaymentPurse,
                    system_module.clone(),
                    RuntimeArgs::default(),
                    &mut handle_payment_named_keys,
                    handle_payment_extra_keys.as_slice(),
                    handle_payment_base_key,
                    &account,
                    authorization_keys.clone(),
                    blocktime,
                    deploy_item.deploy_hash,
                    gas_limit,
                    protocol_version,
                    correlation_id,
                    Rc::clone(&tracking_copy),
                    Phase::Payment,
                    protocol_data,
                    SystemContractCache::clone(&self.system_contract_cache),
                );

            let payment_uref = match payment_uref {
                Some(payment_uref) => payment_uref,
                None => return Ok(make_charged_execution_failure(Error::InsufficientPayment)),
            };

            if let Some(error) = get_payment_purse_result.take_error() {
                return Ok(make_charged_execution_failure(error));
            }

            // Create a new arguments to transfer cost of wasmless transfer into the payment purse.

            let new_transfer_args = TransferArgs::new(
                transfer_args.to(),
                transfer_args.source(),
                payment_uref,
                wasmless_transfer_motes.value(),
                transfer_args.arg_id(),
            );

            let runtime_args = match RuntimeArgs::try_from(new_transfer_args) {
                Ok(runtime_args) => runtime_args,
                Err(error) => return Ok(make_charged_execution_failure(Error::Exec(error.into()))),
            };

            let (actual_result, payment_result): (Option<Result<(), u8>>, ExecutionResult) =
                executor.exec_system_contract(
                    DirectSystemContractCall::Transfer,
                    system_module.clone(),
                    runtime_args,
                    &mut mint_named_keys,
                    mint_extra_keys.as_slice(),
                    mint_base_key,
                    &account,
                    authorization_keys.clone(),
                    blocktime,
                    deploy_item.deploy_hash,
                    gas_limit,
                    protocol_version,
                    correlation_id,
                    Rc::clone(&tracking_copy),
                    Phase::Payment,
                    protocol_data,
                    SystemContractCache::clone(&self.system_contract_cache),
                );

            if let Some(error) = payment_result.as_error().cloned() {
                return Ok(make_charged_execution_failure(error));
            }

            let transfer_result = match actual_result {
                Some(Ok(())) => Ok(()),
                Some(Err(mint_error)) => match mint::Error::try_from(mint_error) {
                    Ok(mint_error) => Err(ApiError::from(mint_error)),
                    Err(_) => Err(ApiError::Transfer),
                },
                None => Err(ApiError::Transfer),
            };

            if let Err(error) = transfer_result {
                return Ok(make_charged_execution_failure(Error::Exec(
                    ExecError::Revert(error),
                )));
            }

            let payment_purse_balance = {
                let payment_purse_balance_key = match tracking_copy
                    .borrow_mut()
                    .get_purse_balance_key(correlation_id, Key::URef(payment_uref))
                {
                    Ok(payment_purse_balance_key) => payment_purse_balance_key,
                    Err(error) => return Ok(make_charged_execution_failure(Error::Exec(error))),
                };

                match tracking_copy
                    .borrow_mut()
                    .get_purse_balance(correlation_id, payment_purse_balance_key)
                {
                    Ok(payment_purse_balance) => payment_purse_balance,
                    Err(error) => return Ok(make_charged_execution_failure(Error::Exec(error))),
                }
            };

            // Wasmless transfer payment code pre & post conditions:
            // (a) payment purse should be empty before the payment operation
            // (b) after executing payment code it's balance has to be equal to the wasmless gas
            // cost price

            let payment_gas =
                match Gas::from_motes(payment_purse_balance, WASMLESS_TRANSFER_FIXED_GAS_PRICE) {
                    Some(gas) => gas,
                    None => {
                        return Ok(make_charged_execution_failure(Error::GasConversionOverflow))
                    }
                };

            debug_assert_eq!(payment_gas, wasmless_transfer_gas_cost);

            // This assumes the cost incurred is already denominated in gas

            payment_result.with_cost(payment_gas)
        };

        let runtime_args = match RuntimeArgs::try_from(transfer_args) {
            Ok(runtime_args) => runtime_args,
            Err(error) => {
                return Ok(make_charged_execution_failure(
                    ExecError::from(error).into(),
                ))
            }
        };

        let (_, mut session_result): (Option<Result<(), u8>>, ExecutionResult) = executor
            .exec_system_contract(
                DirectSystemContractCall::Transfer,
                system_module.clone(),
                runtime_args,
                &mut mint_named_keys,
                mint_extra_keys.as_slice(),
                mint_base_key,
                &account,
                authorization_keys.clone(),
                blocktime,
                deploy_item.deploy_hash,
                gas_limit,
                protocol_version,
                correlation_id,
                Rc::clone(&tracking_copy),
                Phase::Session,
                protocol_data,
                SystemContractCache::clone(&self.system_contract_cache),
            );

        // User is already charged fee for wasmless contract, and we need to make sure we will not
        // charge for anything that happens while calling transfer entrypoint.
        session_result = session_result.with_cost(Gas::default());

        let finalize_result = {
            let handle_payment_args = {
                // Gas spent during payment code execution
                let finalize_cost_motes = {
                    // A case where payment_result.cost() is different than wasmless transfer cost
                    // is considered a programming error.
                    debug_assert_eq!(payment_result.cost(), wasmless_transfer_gas_cost);
                    wasmless_transfer_motes
                };

                let account = deploy_item.address;
                let maybe_runtime_args = RuntimeArgs::try_new(|args| {
                    args.insert(handle_payment::ARG_AMOUNT, finalize_cost_motes.value())?;
                    args.insert(handle_payment::ARG_ACCOUNT, account)?;
                    args.insert(handle_payment::ARG_TARGET, proposer_purse)?;
                    Ok(())
                });

                match maybe_runtime_args {
                    Ok(runtime_args) => runtime_args,
                    Err(error) => {
                        let exec_error = ExecError::from(error);
                        return Ok(ExecutionResult::precondition_failure(exec_error.into()));
                    }
                }
            };

            let system_account = Account::new(
                PublicKey::System.to_account_hash(),
                Default::default(),
                URef::new(Default::default(), AccessRights::READ_ADD_WRITE),
                Default::default(),
                Default::default(),
            );

            let tc = tracking_copy.borrow();
            let finalization_tc = Rc::new(RefCell::new(tc.fork()));

            let (_ret, finalize_result): (Option<()>, ExecutionResult) = executor
                .exec_system_contract(
                    DirectSystemContractCall::FinalizePayment,
                    system_module,
                    handle_payment_args,
                    &mut handle_payment_named_keys,
                    Default::default(),
                    Key::from(protocol_data.handle_payment()),
                    &system_account,
                    authorization_keys,
                    blocktime,
                    deploy_item.deploy_hash,
                    gas_limit,
                    protocol_version,
                    correlation_id,
                    finalization_tc,
                    Phase::FinalizePayment,
                    protocol_data,
                    SystemContractCache::clone(&self.system_contract_cache),
                );

            finalize_result
        };

        // Create + persist deploy info.
        {
            let transfers = session_result.transfers();
            let cost = wasmless_transfer_gas_cost.value();
            let deploy_info = DeployInfo::new(
                deploy_item.deploy_hash,
                &transfers,
                account.account_hash(),
                account.main_purse(),
                cost,
            );
            tracking_copy.borrow_mut().write(
                Key::DeployInfo(deploy_item.deploy_hash),
                StoredValue::DeployInfo(deploy_info),
            );
        }

        if session_result.is_success() {
            session_result = session_result.with_effect(tracking_copy.borrow_mut().effect())
        }

        let mut execution_result_builder = ExecutionResultBuilder::new();
        execution_result_builder.set_payment_execution_result(payment_result);
        execution_result_builder.set_session_execution_result(session_result);
        execution_result_builder.set_finalize_execution_result(finalize_result);

        let execution_result = execution_result_builder
            .build(tracking_copy.borrow().reader(), correlation_id)
            .expect("ExecutionResultBuilder not initialized properly");

        Ok(execution_result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn deploy(
        &self,
        correlation_id: CorrelationId,
        executor: &Executor,
        protocol_version: ProtocolVersion,
        prestate_hash: Blake2bHash,
        blocktime: BlockTime,
        deploy_item: DeployItem,
        proposer: PublicKey,
    ) -> Result<ExecutionResult, Error> {
        // spec: https://casperlabs.atlassian.net/wiki/spaces/EN/pages/123404576/Payment+code+execution+specification

        // Obtain current protocol data for given version
        // do this first, as there is no reason to proceed if protocol version is invalid
        let protocol_data = match self.state.get_protocol_data(protocol_version) {
            Ok(Some(protocol_data)) => protocol_data,
            Ok(None) => {
                let error = Error::InvalidProtocolVersion(protocol_version);
                return Ok(ExecutionResult::precondition_failure(error));
            }
            Err(error) => {
                return Ok(ExecutionResult::precondition_failure(Error::Exec(
                    error.into(),
                )));
            }
        };

        let preprocessor = {
            let wasm_config = protocol_data.wasm_config();
            Preprocessor::new(*wasm_config)
        };

        // Create tracking copy (which functions as a deploy context)
        // validation_spec_2: prestate_hash check
        // do this second; as there is no reason to proceed if the prestate hash is invalid
        let tracking_copy = match self.tracking_copy(prestate_hash) {
            Err(error) => return Ok(ExecutionResult::precondition_failure(error)),
            Ok(None) => return Err(Error::RootNotFound(prestate_hash)),
            Ok(Some(tracking_copy)) => Rc::new(RefCell::new(tracking_copy)),
        };

        let system_module = {
            match tracking_copy.borrow_mut().get_system_module(&preprocessor) {
                Ok(module) => module,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            }
        };

        // vestigial system_contract_cache
        self.system_contract_cache
            .initialize_with_protocol_data(&protocol_data, &system_module);

        let base_key = Key::Account(deploy_item.address);

        // Get addr bytes from `address` (which is actually a Key)
        // validation_spec_3: account validity
        let account_hash = match base_key.into_account() {
            Some(account_addr) => account_addr,
            None => {
                return Ok(ExecutionResult::precondition_failure(
                    error::Error::Authorization,
                ));
            }
        };

        let authorization_keys = deploy_item.authorization_keys;

        // Get account from tracking copy
        // validation_spec_3: account validity
        let account = match self.get_authorized_account(
            correlation_id,
            account_hash,
            &authorization_keys,
            Rc::clone(&tracking_copy),
        ) {
            Ok(account) => account,
            Err(e) => return Ok(ExecutionResult::precondition_failure(e)),
        };

        let session = deploy_item.session;
        let payment = deploy_item.payment;
        let deploy_hash = deploy_item.deploy_hash;

        // Create session code `A` from provided session bytes
        // validation_spec_1: valid wasm bytes
        // we do this upfront as there is no reason to continue if session logic is invalid
        let session_metadata = match session.get_deploy_metadata(
            Rc::clone(&tracking_copy),
            &account,
            correlation_id,
            &preprocessor,
            &protocol_version,
            &protocol_data,
            Phase::Session,
        ) {
            Ok(metadata) => metadata,
            Err(error) => {
                return Ok(ExecutionResult::precondition_failure(error));
            }
        };

        // Get account main purse balance key
        // validation_spec_5: account main purse minimum balance
        let account_main_purse_balance_key: Key = {
            let account_key = Key::URef(account.main_purse());
            match tracking_copy
                .borrow_mut()
                .get_purse_balance_key(correlation_id, account_key)
            {
                Ok(key) => key,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            }
        };

        // Get account main purse balance to enforce precondition and in case of forced
        // transfer validation_spec_5: account main purse minimum balance
        let account_main_purse_balance: Motes = match tracking_copy
            .borrow_mut()
            .get_purse_balance(correlation_id, account_main_purse_balance_key)
        {
            Ok(balance) => balance,
            Err(error) => return Ok(ExecutionResult::precondition_failure(error.into())),
        };

        let max_payment_cost = Motes::new(*MAX_PAYMENT);

        // Enforce minimum main purse balance validation
        // validation_spec_5: account main purse minimum balance
        if account_main_purse_balance < max_payment_cost {
            return Ok(ExecutionResult::precondition_failure(
                Error::InsufficientPayment,
            ));
        }

        // Finalization is executed by system account (currently genesis account)
        // payment_code_spec_5: system executes finalization
        let system_account = Account::new(
            PublicKey::System.to_account_hash(),
            Default::default(),
            URef::new(Default::default(), AccessRights::READ_ADD_WRITE),
            Default::default(),
            Default::default(),
        );

        // [`ExecutionResultBuilder`] handles merging of multiple execution results
        let mut execution_result_builder = execution_result::ExecutionResultBuilder::new();

        // Execute provided payment code
        let payment_result = {
            // payment_code_spec_1: init pay environment w/ gas limit == (max_payment_cost /
            // gas_price)
            let payment_gas_limit = match Gas::from_motes(max_payment_cost, deploy_item.gas_price) {
                Some(gas) => gas,
                None => {
                    return Ok(ExecutionResult::precondition_failure(
                        Error::GasConversionOverflow,
                    ))
                }
            };

            // Create payment code module from bytes
            // validation_spec_1: valid wasm bytes
            let phase = Phase::Payment;
            let payment_metadata = match payment.get_deploy_metadata(
                Rc::clone(&tracking_copy),
                &account,
                correlation_id,
                &preprocessor,
                &protocol_version,
                &protocol_data,
                phase,
            ) {
                Ok(metadata) => metadata,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error));
                }
            };

            // payment_code_spec_2: execute payment code
            let (
                payment_module,
                payment_base_key,
                mut payment_named_keys,
                payment_package,
                payment_entry_point,
                is_standard_payment,
            ) = match payment_metadata {
                DeployMetadata::System {
                    contract_package,
                    entry_point,
                    ..
                } => (
                    system_module.clone(),
                    base_key,                     // this is account key
                    account.named_keys().clone(), // standard payment uses account keys
                    contract_package,
                    entry_point,
                    true,
                ),
                DeployMetadata::Session {
                    module,
                    contract_package,
                    entry_point,
                } => (
                    module,
                    base_key, // this is account key
                    account.named_keys().clone(),
                    contract_package,
                    entry_point,
                    false,
                ),
                DeployMetadata::Contract {
                    module,
                    base_key,
                    contract,
                    contract_package,
                    entry_point,
                } => (
                    module,
                    base_key, // this is contract key
                    contract.named_keys().clone(),
                    contract_package,
                    entry_point,
                    false,
                ),
            };

            let payment_args = payment.args().clone();
            let system_contract_cache = SystemContractCache::clone(&self.system_contract_cache);

            if is_standard_payment {
                executor.exec_standard_payment(
                    system_module.clone(),
                    payment_args,
                    payment_base_key,
                    &account,
                    &mut payment_named_keys,
                    authorization_keys.clone(),
                    blocktime,
                    deploy_hash,
                    payment_gas_limit,
                    protocol_version,
                    correlation_id,
                    Rc::clone(&tracking_copy),
                    phase,
                    protocol_data,
                    system_contract_cache,
                )
            } else {
                executor.exec(
                    payment_module,
                    payment_entry_point,
                    payment_args,
                    payment_base_key,
                    &account,
                    &mut payment_named_keys,
                    authorization_keys.clone(),
                    blocktime,
                    deploy_hash,
                    payment_gas_limit,
                    protocol_version,
                    correlation_id,
                    Rc::clone(&tracking_copy),
                    phase,
                    protocol_data,
                    system_contract_cache,
                    &payment_package,
                )
            }
        };

        debug!("Payment result: {:?}", payment_result);

        let payment_result_cost = payment_result.cost();
        // payment_code_spec_3: fork based upon payment purse balance and cost of
        // payment code execution
        let payment_purse_balance: Motes = {
            // Get handle payment system contract details
            // payment_code_spec_6: system contract validity
            let handle_payment_contract = match tracking_copy
                .borrow_mut()
                .get_contract(correlation_id, protocol_data.handle_payment())
            {
                Ok(contract) => contract,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            };

            // Get payment purse Key from handle payment contract
            // payment_code_spec_6: system contract validity
            let payment_purse_key: Key = match handle_payment_contract
                .named_keys()
                .get(handle_payment::PAYMENT_PURSE_KEY)
            {
                Some(key) => *key,
                None => return Ok(ExecutionResult::precondition_failure(Error::Deploy)),
            };

            let purse_balance_key = match tracking_copy
                .borrow_mut()
                .get_purse_balance_key(correlation_id, payment_purse_key)
            {
                Ok(key) => key,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            };

            match tracking_copy
                .borrow_mut()
                .get_purse_balance(correlation_id, purse_balance_key)
            {
                Ok(balance) => balance,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            }
        };

        // the proposer of the block this deploy is in receives the gas from this deploy execution
        let proposer_purse = {
            let proposer_account: Account = match tracking_copy
                .borrow_mut()
                .get_account(correlation_id, AccountHash::from(&proposer))
            {
                Ok(account) => account,
                Err(error) => {
                    return Ok(ExecutionResult::precondition_failure(error.into()));
                }
            };
            proposer_account.main_purse()
        };

        if let Some(forced_transfer) =
            payment_result.check_forced_transfer(payment_purse_balance, deploy_item.gas_price)
        {
            // Get rewards purse balance key
            // payment_code_spec_6: system contract validity
            let proposer_main_purse_balance_key = {
                // Get reward purse Key from handle payment contract
                // payment_code_spec_6: system contract validity
                match tracking_copy
                    .borrow_mut()
                    .get_purse_balance_key(correlation_id, proposer_purse.into())
                {
                    Ok(key) => key,
                    Err(error) => {
                        return Ok(ExecutionResult::precondition_failure(error.into()));
                    }
                }
            };

            let error = match forced_transfer {
                ForcedTransferResult::InsufficientPayment => Error::InsufficientPayment,
                ForcedTransferResult::GasConversionOverflow => Error::GasConversionOverflow,
                ForcedTransferResult::PaymentFailure => payment_result
                    .take_error()
                    .unwrap_or(Error::InsufficientPayment),
            };

            let gas_cost = match Gas::from_motes(max_payment_cost, deploy_item.gas_price) {
                Some(gas) => gas,
                None => {
                    return Ok(ExecutionResult::precondition_failure(
                        Error::GasConversionOverflow,
                    ))
                }
            };

            match ExecutionResult::new_payment_code_error(
                error,
                max_payment_cost,
                account_main_purse_balance,
                gas_cost,
                account_main_purse_balance_key,
                proposer_main_purse_balance_key,
            ) {
                Ok(execution_result) => return Ok(execution_result),
                Err(error) => {
                    let exec_error = ExecError::from(error);
                    return Ok(ExecutionResult::precondition_failure(exec_error.into()));
                }
            }
        };

        // Transfer the contents of the rewards purse to block proposer
        execution_result_builder.set_payment_execution_result(payment_result);

        // Begin session logic handling
        let post_payment_tracking_copy = tracking_copy.borrow();
        let session_tracking_copy = Rc::new(RefCell::new(post_payment_tracking_copy.fork()));

        // session_code_spec_2: execute session code
        let (
            session_module,
            session_base_key,
            mut session_named_keys,
            session_package,
            session_entry_point,
        ) = match session_metadata {
            DeployMetadata::System {
                base_key,
                contract,
                contract_package,
                entry_point,
            } => {
                (
                    system_module.clone(),
                    base_key, // this is contract key
                    contract.named_keys().clone(),
                    contract_package,
                    entry_point,
                )
            }
            DeployMetadata::Session {
                module,
                contract_package,
                entry_point,
            } => (
                module,
                base_key,
                account.named_keys().clone(),
                contract_package,
                entry_point,
            ),
            DeployMetadata::Contract {
                module,
                base_key,
                contract,
                contract_package,
                entry_point,
            } => (
                module,
                base_key,
                contract.named_keys().clone(),
                contract_package,
                entry_point,
            ),
        };

        let session_args = session.args().clone();
        let mut session_result = {
            // payment_code_spec_3_b_i: if (balance of handle payment pay purse) >= (gas spent
            // during payment code execution) * gas_price, yes session
            // session_code_spec_1: gas limit = ((balance of handle payment payment purse) /
            // gas_price)
            // - (gas spent during payment execution)
            let session_gas_limit: Gas =
                match Gas::from_motes(payment_purse_balance, deploy_item.gas_price)
                    .and_then(|gas| gas.checked_sub(payment_result_cost))
                {
                    Some(gas) => gas,
                    None => {
                        return Ok(ExecutionResult::precondition_failure(
                            Error::GasConversionOverflow,
                        ))
                    }
                };
            let system_contract_cache = SystemContractCache::clone(&self.system_contract_cache);

            executor.exec(
                session_module,
                session_entry_point,
                session_args,
                session_base_key,
                &account,
                &mut session_named_keys,
                authorization_keys.clone(),
                blocktime,
                deploy_hash,
                session_gas_limit,
                protocol_version,
                correlation_id,
                Rc::clone(&session_tracking_copy),
                Phase::Session,
                protocol_data,
                system_contract_cache,
                &session_package,
            )
        };
        debug!("Session result: {:?}", session_result);

        // Create + persist deploy info.
        {
            let transfers = session_result.transfers();
            let cost = payment_result_cost.value() + session_result.cost().value();
            let deploy_info = DeployInfo::new(
                deploy_hash,
                &transfers,
                account.account_hash(),
                account.main_purse(),
                cost,
            );
            session_tracking_copy.borrow_mut().write(
                Key::DeployInfo(deploy_hash),
                StoredValue::DeployInfo(deploy_info),
            );
        }

        let post_session_rc = if session_result.is_failure() {
            // If session code fails we do not include its effects,
            // so we start again from the post-payment state.
            Rc::new(RefCell::new(post_payment_tracking_copy.fork()))
        } else {
            session_result = session_result.with_effect(session_tracking_copy.borrow().effect());
            session_tracking_copy
        };

        // NOTE: session_code_spec_3: (do not include session execution effects in
        // results) is enforced in execution_result_builder.build()
        execution_result_builder.set_session_execution_result(session_result);

        // payment_code_spec_5: run finalize process
        let finalize_result: ExecutionResult = {
            let post_session_tc = post_session_rc.borrow();
            let finalization_tc = Rc::new(RefCell::new(post_session_tc.fork()));

            let handle_payment_args = {
                //((gas spent during payment code execution) + (gas spent during session code execution)) * gas_price
                let finalize_cost_motes = match Motes::from_gas(execution_result_builder.total_cost(), deploy_item.gas_price) {
                    Some(motes) => motes,
                    None => return Ok(ExecutionResult::precondition_failure(Error::GasConversionOverflow)),
                };

                let maybe_runtime_args = RuntimeArgs::try_new(|args| {
                    args.insert(handle_payment::ARG_AMOUNT, finalize_cost_motes.value())?;
                    args.insert(handle_payment::ARG_ACCOUNT, account_hash)?;
                    args.insert(handle_payment::ARG_TARGET, proposer_purse)?;
                    Ok(())
                });
                match maybe_runtime_args {
                    Ok(runtime_args) => runtime_args,
                    Err(error) => {
                        let exec_error = ExecError::from(error);
                        return Ok(ExecutionResult::precondition_failure(exec_error.into()));
                    }
                }
            };

            // The Handle Payment keys may have changed because of effects during payment and/or
            // session, so we need to look them up again from the tracking copy
            let handle_payment_contract = match finalization_tc
                .borrow_mut()
                .get_contract(correlation_id, protocol_data.handle_payment())
            {
                Ok(info) => info,
                Err(error) => return Ok(ExecutionResult::precondition_failure(error.into())),
            };

            let mut handle_payment_keys = handle_payment_contract.named_keys().to_owned();

            let gas_limit = Gas::new(U512::from(std::u64::MAX));
            let system_contract_cache = SystemContractCache::clone(&self.system_contract_cache);

            let (_ret, finalize_result): (Option<()>, ExecutionResult) = executor
                .exec_system_contract(
                    DirectSystemContractCall::FinalizePayment,
                    system_module,
                    handle_payment_args,
                    &mut handle_payment_keys,
                    Default::default(),
                    Key::from(protocol_data.handle_payment()),
                    &system_account,
                    authorization_keys,
                    blocktime,
                    deploy_hash,
                    gas_limit,
                    protocol_version,
                    correlation_id,
                    finalization_tc,
                    Phase::FinalizePayment,
                    protocol_data,
                    system_contract_cache,
                );

            finalize_result
        };

        execution_result_builder.set_finalize_execution_result(finalize_result);

        // We panic here to indicate that the builder was not used properly.
        let ret = execution_result_builder
            .build(tracking_copy.borrow().reader(), correlation_id)
            .expect("ExecutionResultBuilder not initialized properly");

        // NOTE: payment_code_spec_5_a is enforced in execution_result_builder.build()
        // payment_code_spec_6: return properly combined set of transforms and
        // appropriate error
        Ok(ret)
    }

    pub fn apply_effect(
        &self,
        correlation_id: CorrelationId,
        pre_state_hash: Blake2bHash,
        effects: AdditiveMap<Key, Transform>,
    ) -> Result<CommitResult, Error>
    where
        Error: From<S::Error>,
    {
        self.state
            .commit(correlation_id, pre_state_hash, effects)
            .map_err(Error::from)
    }

    pub fn read_trie(
        &self,
        correlation_id: CorrelationId,
        trie_key: Blake2bHash,
    ) -> Result<Option<Trie<Key, StoredValue>>, Error>
    where
        Error: From<S::Error>,
    {
        self.state
            .read_trie(correlation_id, &trie_key)
            .map_err(Error::from)
    }

    pub fn put_trie_and_find_missing_descendant_trie_keys(
        &self,
        correlation_id: CorrelationId,
        trie: &Trie<Key, StoredValue>,
    ) -> Result<Vec<Blake2bHash>, Error>
    where
        Error: From<S::Error>,
    {
        let inserted_trie_key = self.state.put_trie(correlation_id, trie)?;
        let missing_descendant_trie_keys = self
            .state
            .missing_trie_keys(correlation_id, vec![inserted_trie_key])?;
        Ok(missing_descendant_trie_keys)
    }

    pub fn missing_trie_keys(
        &self,
        correlation_id: CorrelationId,
        trie_keys: Vec<Blake2bHash>,
    ) -> Result<Vec<Blake2bHash>, Error>
    where
        Error: From<S::Error>,
    {
        self.state
            .missing_trie_keys(correlation_id, trie_keys)
            .map_err(Error::from)
    }

    /// Obtains validator weights for given era.
    pub fn get_era_validators(
        &self,
        correlation_id: CorrelationId,
        get_era_validators_request: GetEraValidatorsRequest,
    ) -> Result<EraValidators, GetEraValidatorsError> {
        let protocol_version = get_era_validators_request.protocol_version();

        let tracking_copy = match self.tracking_copy(get_era_validators_request.state_hash())? {
            Some(tracking_copy) => Rc::new(RefCell::new(tracking_copy)),
            None => return Err(GetEraValidatorsError::RootNotFound),
        };

        let protocol_data = match self.get_protocol_data(protocol_version)? {
            Some(protocol_data) => protocol_data,
            None => return Err(Error::InvalidProtocolVersion(protocol_version).into()),
        };

        let wasm_config = protocol_data.wasm_config();

        let preprocessor = Preprocessor::new(*wasm_config);

        let auction_contract: Contract = tracking_copy
            .borrow_mut()
            .get_contract(correlation_id, protocol_data.auction())
            .map_err(Error::from)?;

        let system_module = {
            tracking_copy
                .borrow_mut()
                .get_system_module(&preprocessor)
                .map_err(Error::from)?
        };

        let executor = Executor::new(self.config);

        let mut named_keys = auction_contract.named_keys().to_owned();
        let base_key = Key::from(protocol_data.auction());
        let gas_limit = Gas::new(U512::from(std::u64::MAX));
        let virtual_system_account = {
            let named_keys = NamedKeys::new();
            let purse = URef::new(Default::default(), AccessRights::READ_ADD_WRITE);
            Account::create(PublicKey::System.to_account_hash(), named_keys, purse)
        };
        let authorization_keys = BTreeSet::from_iter(vec![PublicKey::System.to_account_hash()]);
        let blocktime = BlockTime::default();
        let deploy_hash = {
            // seeds address generator w/ protocol version
            let bytes: Vec<u8> = get_era_validators_request
                .protocol_version()
                .value()
                .into_bytes()
                .map_err(Error::from)?
                .to_vec();
            DeployHash::new(Blake2bHash::new(&bytes).value())
        };

        let (era_validators, execution_result): (Option<EraValidators>, ExecutionResult) = executor
            .exec_system_contract(
                DirectSystemContractCall::GetEraValidators,
                system_module,
                RuntimeArgs::new(),
                &mut named_keys,
                Default::default(),
                base_key,
                &virtual_system_account,
                authorization_keys,
                blocktime,
                deploy_hash,
                gas_limit,
                protocol_version,
                correlation_id,
                Rc::clone(&tracking_copy),
                Phase::Session,
                protocol_data,
                SystemContractCache::clone(&self.system_contract_cache),
            );

        if let Some(error) = execution_result.take_error() {
            return Err(error.into());
        }

        match era_validators {
            None => Err(GetEraValidatorsError::EraValidatorsMissing),
            Some(era_validators) => Ok(era_validators),
        }
    }

    pub fn get_bids(
        &self,
        correlation_id: CorrelationId,
        get_bids_request: GetBidsRequest,
    ) -> Result<GetBidsResult, Error> {
        let tracking_copy = match self.tracking_copy(get_bids_request.state_hash())? {
            Some(tracking_copy) => Rc::new(RefCell::new(tracking_copy)),
            None => return Ok(GetBidsResult::RootNotFound),
        };

        let mut tracking_copy = tracking_copy.borrow_mut();

        let bid_keys = tracking_copy
            .get_keys(correlation_id, &KeyTag::Bid)
            .map_err(|err| Error::Exec(err.into()))?;

        let mut bids = BTreeMap::new();

        for key in bid_keys.iter() {
            if let Some(StoredValue::Bid(bid)) =
                tracking_copy.get(correlation_id, key).map_err(Into::into)?
            {
                bids.insert(bid.validator_public_key().clone(), *bid);
            };
        }

        Ok(GetBidsResult::Success { bids })
    }

    pub fn commit_step(
        &self,
        correlation_id: CorrelationId,
        step_request: StepRequest,
    ) -> Result<StepResult, Error> {
        let protocol_data = match self.state.get_protocol_data(step_request.protocol_version) {
            Ok(Some(protocol_data)) => protocol_data,
            Ok(None) => {
                return Ok(StepResult::InvalidProtocolVersion);
            }
            Err(error) => return Ok(StepResult::GetProtocolDataError(Error::Exec(error.into()))),
        };

        let tracking_copy = match self.tracking_copy(step_request.pre_state_hash) {
            Err(error) => return Ok(StepResult::TrackingCopyError(error)),
            Ok(None) => return Ok(StepResult::RootNotFound),
            Ok(Some(tracking_copy)) => Rc::new(RefCell::new(tracking_copy)),
        };

        let executor = Executor::new(self.config);

        let preprocessor = {
            let wasm_config = protocol_data.wasm_config();
            Preprocessor::new(*wasm_config)
        };

        let auction_hash = protocol_data.auction();

        let auction_contract = match tracking_copy
            .borrow_mut()
            .get_contract(correlation_id, auction_hash)
        {
            Ok(contract) => contract,
            Err(error) => {
                return Ok(StepResult::GetContractError(error.into()));
            }
        };

        let system_module = match tracking_copy.borrow_mut().get_system_module(&preprocessor) {
            Ok(module) => module,
            Err(error) => {
                return Ok(StepResult::GetSystemModuleError(error.into()));
            }
        };

        self.system_contract_cache
            .initialize_with_protocol_data(&protocol_data, &system_module);

        let system_account_addr = PublicKey::System.to_account_hash();

        let virtual_system_account = {
            let named_keys = NamedKeys::new();
            let purse = URef::new(Default::default(), AccessRights::READ_ADD_WRITE);
            Account::create(system_account_addr, named_keys, purse)
        };
        let authorization_keys = {
            let mut ret = BTreeSet::new();
            ret.insert(system_account_addr);
            ret
        };
        let mut named_keys = auction_contract.named_keys().to_owned();
        let gas_limit = Gas::new(U512::from(std::u64::MAX));
        let deploy_hash = {
            // seeds address generator w/ protocol version
            let bytes: Vec<u8> = step_request.protocol_version.value().into_bytes()?.to_vec();
            DeployHash::new(Blake2bHash::new(&bytes).value())
        };

        let base_key = Key::from(protocol_data.auction());

        let reward_factors = match step_request.reward_factors() {
            Ok(reward_factors) => reward_factors,
            Err(error) => {
                error!(
                    "failed to deserialize reward factors: {}",
                    error.to_string()
                );
                return Ok(StepResult::Serialization(error));
            }
        };

        let reward_args = {
            let maybe_runtime_args = RuntimeArgs::try_new(|args| {
                args.insert(ARG_REWARD_FACTORS, reward_factors)?;
                Ok(())
            });

            match maybe_runtime_args {
                Ok(runtime_args) => runtime_args,
                Err(error) => return Ok(StepResult::CLValueError(error)),
            }
        };

        let (_, execution_result): (Option<()>, ExecutionResult) = executor.exec_system_contract(
            DirectSystemContractCall::DistributeRewards,
            system_module.clone(),
            reward_args,
            &mut named_keys,
            Default::default(),
            base_key,
            &virtual_system_account,
            authorization_keys.clone(),
            BlockTime::default(),
            deploy_hash,
            gas_limit,
            step_request.protocol_version,
            correlation_id,
            Rc::clone(&tracking_copy),
            Phase::Session,
            protocol_data,
            SystemContractCache::clone(&self.system_contract_cache),
        );

        if let Some(exec_error) = execution_result.take_error() {
            return Ok(StepResult::DistributeError(exec_error));
        }

        let slashed_validators = match step_request.slashed_validators() {
            Ok(slashed_validators) => slashed_validators,
            Err(error) => {
                error!(
                    "failed to deserialize validator_ids for slashing: {}",
                    error.to_string()
                );
                return Ok(StepResult::Serialization(error));
            }
        };

        let slash_args = {
            let mut runtime_args = RuntimeArgs::new();
            runtime_args
                .insert(ARG_VALIDATOR_PUBLIC_KEYS, slashed_validators)
                .map_err(|e| Error::Exec(e.into()))?;
            runtime_args
        };

        let (_, execution_result): (Option<()>, ExecutionResult) = executor.exec_system_contract(
            DirectSystemContractCall::Slash,
            system_module.clone(),
            slash_args,
            &mut named_keys,
            Default::default(),
            base_key,
            &virtual_system_account,
            authorization_keys.clone(),
            BlockTime::default(),
            deploy_hash,
            gas_limit,
            step_request.protocol_version,
            correlation_id,
            Rc::clone(&tracking_copy),
            Phase::Session,
            protocol_data,
            SystemContractCache::clone(&self.system_contract_cache),
        );

        if let Some(exec_error) = execution_result.take_error() {
            return Ok(StepResult::SlashingError(exec_error));
        }

        if step_request.run_auction {
            let run_auction_args = {
                let maybe_runtime_args = RuntimeArgs::try_new(|args| {
                    args.insert(
                        ARG_ERA_END_TIMESTAMP_MILLIS,
                        step_request.era_end_timestamp_millis,
                    )?;
                    args.insert(
                        ARG_EVICTED_VALIDATORS,
                        step_request
                            .evict_items
                            .iter()
                            .map(|item| item.validator_id.clone())
                            .collect::<Vec<PublicKey>>(),
                    )?;
                    Ok(())
                });

                match maybe_runtime_args {
                    Ok(runtime_args) => runtime_args,
                    Err(error) => return Ok(StepResult::CLValueError(error)),
                }
            };

            let (_, execution_result): (Option<()>, ExecutionResult) = executor
                .exec_system_contract(
                    DirectSystemContractCall::RunAuction,
                    system_module,
                    run_auction_args,
                    &mut named_keys,
                    Default::default(),
                    base_key,
                    &virtual_system_account,
                    authorization_keys,
                    BlockTime::default(),
                    deploy_hash,
                    gas_limit,
                    step_request.protocol_version,
                    correlation_id,
                    Rc::clone(&tracking_copy),
                    Phase::Session,
                    protocol_data,
                    SystemContractCache::clone(&self.system_contract_cache),
                );

            if let Some(exec_error) = execution_result.take_error() {
                return Ok(StepResult::AuctionError(exec_error));
            }
        }

        let effects = tracking_copy.borrow().effect();

        // commit
        let commit_result = self
            .state
            .commit(
                correlation_id,
                step_request.pre_state_hash,
                effects.transforms.clone(),
            )
            .map_err(Into::into)?;

        let post_state_hash = match commit_result {
            CommitResult::Success { state_root } => state_root,
            CommitResult::RootNotFound => return Ok(StepResult::RootNotFound),
            CommitResult::KeyNotFound(key) => return Ok(StepResult::KeyNotFound(key)),
            CommitResult::TypeMismatch(type_mismatch) => {
                return Ok(StepResult::TypeMismatch(type_mismatch))
            }
            CommitResult::Serialization(bytesrepr_error) => {
                return Ok(StepResult::Serialization(bytesrepr_error))
            }
        };

        let next_era_validators = {
            let mut era_validators = match self.get_era_validators(
                correlation_id,
                GetEraValidatorsRequest::new(post_state_hash, step_request.protocol_version),
            ) {
                Ok(era_validators) => era_validators,
                Err(error) => {
                    return Ok(StepResult::GetEraValidatorsError(error));
                }
            };

            let era_id = &step_request.next_era_id;
            match era_validators.remove(era_id) {
                Some(validator_weights) => validator_weights,
                None => {
                    return Ok(StepResult::EraValidatorsMissing(*era_id));
                }
            }
        };

        Ok(StepResult::Success {
            post_state_hash,
            next_era_validators,
            execution_effect: effects,
        })
    }

    pub fn get_balance(
        &self,
        correlation_id: CorrelationId,
        state_hash: Blake2bHash,
        public_key: PublicKey,
    ) -> Result<BalanceResult, Error> {
        // Look up the account, get the main purse, and then do the existing balance check
        let tracking_copy = match self.tracking_copy(state_hash) {
            Ok(Some(tracking_copy)) => Rc::new(RefCell::new(tracking_copy)),
            Ok(None) => return Ok(BalanceResult::RootNotFound),
            Err(error) => return Err(error),
        };

        let account_addr = public_key.to_account_hash();

        let account = match tracking_copy
            .borrow_mut()
            .get_account(correlation_id, account_addr)
        {
            Ok(account) => account,
            Err(error) => return Err(error.into()),
        };

        let main_purse_balance_key = {
            let main_purse = account.main_purse();
            match tracking_copy
                .borrow()
                .get_purse_balance_key(correlation_id, main_purse.into())
            {
                Ok(balance_key) => balance_key,
                Err(error) => return Err(error.into()),
            }
        };

        let (account_balance, proof) = match tracking_copy
            .borrow()
            .get_purse_balance_with_proof(correlation_id, main_purse_balance_key)
        {
            Ok((balance, proof)) => (balance, proof),
            Err(error) => return Err(error.into()),
        };

        let proof = Box::new(proof);
        let motes = account_balance.value();
        Ok(BalanceResult::Success { motes, proof })
    }
}
