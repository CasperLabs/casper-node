use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt, iter,
    rc::Rc,
};

use datasize::DataSize;
use num_rational::Ratio;
use num_traits::Zero;
use parity_wasm::elements::Module;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::SYSTEM_ACCOUNT_ADDR;
use crate::{
    core::{
        engine_state::{
            execution_effect::ExecutionEffect, EngineConfig, ExecutionResult, SystemContractCache,
        },
        execution,
        execution::{AddressGenerators, DirectSystemContractCall, Executor},
        tracking_copy::{TrackingCopy, TrackingCopyExt},
    },
    shared::{
        account::Account,
        gas::Gas,
        motes::Motes,
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
        wasm_config::WasmConfig,
        wasm_prep::Preprocessor,
        TypeMismatch,
    },
    storage::{
        global_state::{CommitResult, StateProvider},
        protocol_data::ProtocolData,
    },
};
use casper_types::{
    account::AccountHash,
    auction::{
        Bid, Bids, DelegationRate, EraId, SeigniorageRecipient, SeigniorageRecipients,
        SeigniorageRecipientsSnapshot, UnbondingPurses, ValidatorWeights, ARG_DELEGATION_RATE,
        ARG_DELEGATOR, ARG_DELEGATOR_PUBLIC_KEY, ARG_PUBLIC_KEY, ARG_REWARD_FACTORS,
        ARG_SOURCE_PURSE, ARG_TARGET_PURSE, ARG_UNBOND_PURSE, ARG_VALIDATOR,
        ARG_VALIDATOR_PUBLIC_KEY, AUCTION_DELAY_KEY, BIDS_KEY, DELEGATOR_REWARD_PURSE_KEY,
        ERA_ID_KEY, INITIAL_ERA_ID, LOCKED_FUNDS_PERIOD_KEY, METHOD_ADD_BID, METHOD_DELEGATE,
        METHOD_DISTRIBUTE, METHOD_GET_ERA_VALIDATORS, METHOD_READ_ERA_ID,
        METHOD_READ_SEIGNIORAGE_RECIPIENTS, METHOD_RUN_AUCTION, METHOD_SLASH, METHOD_UNDELEGATE,
        METHOD_WITHDRAW_BID, METHOD_WITHDRAW_DELEGATOR_REWARD, METHOD_WITHDRAW_VALIDATOR_REWARD,
        SEIGNIORAGE_RECIPIENTS_SNAPSHOT_KEY, UNBONDING_DELAY_KEY, UNBONDING_PURSES_KEY,
        VALIDATOR_REWARD_PURSE_KEY, VALIDATOR_SLOTS_KEY,
    },
    bytesrepr::{self, FromBytes, ToBytes},
    contracts::{ContractVersions, DisabledVersions, Groups, NamedKeys, Parameters},
    mint::{
        ARG_AMOUNT, ARG_ID, ARG_PURSE, ARG_ROUND_SEIGNIORAGE_RATE, ARG_SOURCE, ARG_TARGET,
        METHOD_BALANCE, METHOD_CREATE, METHOD_MINT, METHOD_READ_BASE_ROUND_REWARD,
        METHOD_REDUCE_TOTAL_SUPPLY, METHOD_TRANSFER, ROUND_SEIGNIORAGE_RATE_KEY, TOTAL_SUPPLY_KEY,
    },
    proof_of_stake::{
        ARG_ACCOUNT, METHOD_FINALIZE_PAYMENT, METHOD_GET_PAYMENT_PURSE, METHOD_GET_REFUND_PURSE,
        METHOD_SET_REFUND_PURSE,
    },
    runtime_args,
    standard_payment::METHOD_CALL,
    system_contract_errors::mint,
    AccessRights, BlockTime, CLType, CLTyped, CLValue, Contract, ContractHash, ContractPackage,
    ContractPackageHash, ContractWasm, DeployHash, EntryPoint, EntryPointAccess, EntryPointType,
    EntryPoints, Key, Parameter, Phase, ProtocolVersion, PublicKey, RuntimeArgs, SecretKey, URef,
    U512,
};

pub const PLACEHOLDER_KEY: Key = Key::Hash([0u8; 32]);
pub const POS_PAYMENT_PURSE: &str = "pos_payment_purse";

#[derive(Debug, Serialize)]
pub enum GenesisResult {
    RootNotFound,
    KeyNotFound(Key),
    TypeMismatch(TypeMismatch),
    Serialization(bytesrepr::Error),
    Success {
        post_state_hash: Blake2bHash,
        #[serde(skip_serializing)]
        effect: ExecutionEffect,
    },
}

impl fmt::Display for GenesisResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            GenesisResult::RootNotFound => write!(f, "Root not found"),
            GenesisResult::KeyNotFound(key) => write!(f, "Key not found: {}", key),
            GenesisResult::TypeMismatch(type_mismatch) => {
                write!(f, "Type mismatch: {:?}", type_mismatch)
            }
            GenesisResult::Serialization(error) => write!(f, "Serialization error: {:?}", error),
            GenesisResult::Success {
                post_state_hash,
                effect,
            } => write!(f, "Success: {} {:?}", post_state_hash, effect),
        }
    }
}

impl GenesisResult {
    pub fn from_commit_result(commit_result: CommitResult, effect: ExecutionEffect) -> Self {
        match commit_result {
            CommitResult::RootNotFound => GenesisResult::RootNotFound,
            CommitResult::KeyNotFound(key) => GenesisResult::KeyNotFound(key),
            CommitResult::TypeMismatch(type_mismatch) => GenesisResult::TypeMismatch(type_mismatch),
            CommitResult::Serialization(error) => GenesisResult::Serialization(error),
            CommitResult::Success { state_root, .. } => GenesisResult::Success {
                post_state_hash: state_root,
                effect,
            },
        }
    }
}

#[derive(DataSize, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisAccount {
    /// Assumed to be a system account if `public_key` is not specified.
    public_key: Option<PublicKey>,
    account_hash: AccountHash,
    balance: Motes,
    bonded_amount: Motes,
}

impl GenesisAccount {
    pub fn system(balance: Motes, bonded_amount: Motes) -> Self {
        Self {
            public_key: None,
            account_hash: SYSTEM_ACCOUNT_ADDR,
            balance,
            bonded_amount,
        }
    }

    pub fn new(
        public_key: PublicKey,
        account_hash: AccountHash,
        balance: Motes,
        bonded_amount: Motes,
    ) -> Self {
        GenesisAccount {
            public_key: Some(public_key),
            account_hash,
            balance,
            bonded_amount,
        }
    }

    pub fn public_key(&self) -> Option<PublicKey> {
        self.public_key
    }

    pub fn account_hash(&self) -> AccountHash {
        self.account_hash
    }

    pub fn balance(&self) -> Motes {
        self.balance
    }

    pub fn bonded_amount(&self) -> Motes {
        self.bonded_amount
    }

    /// Checks if a given genesis account belongs to a virtual system account,
    pub fn is_system_account(&self) -> bool {
        self.public_key.is_none()
    }

    /// Checks if a given genesis account is a valid genesis validator.
    ///
    /// Genesis validators are the ones with a stake, and are not owned by a virtual system account.
    pub fn is_genesis_validator(&self) -> bool {
        !self.is_system_account() && !self.bonded_amount.is_zero()
    }
}

impl Distribution<GenesisAccount> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisAccount {
        let account_hash = AccountHash::new(rng.gen());

        let public_key = SecretKey::ed25519(rng.gen()).into();

        let mut u512_array = [0u8; 64];
        rng.fill_bytes(u512_array.as_mut());
        let balance = Motes::new(U512::from(u512_array));

        rng.fill_bytes(u512_array.as_mut());
        let bonded_amount = Motes::new(U512::from(u512_array));

        GenesisAccount::new(public_key, account_hash, balance, bonded_amount)
    }
}

impl ToBytes for GenesisAccount {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.public_key.to_bytes()?);
        buffer.extend(self.account_hash.to_bytes()?);
        buffer.extend(self.balance.value().to_bytes()?);
        buffer.extend(self.bonded_amount.value().to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.public_key.serialized_length()
            + self.account_hash.serialized_length()
            + self.balance.value().serialized_length()
            + self.bonded_amount.value().serialized_length()
    }
}

impl FromBytes for GenesisAccount {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (public_key, remainder) = Option::<PublicKey>::from_bytes(bytes)?;
        let (account_hash, remainder) = AccountHash::from_bytes(remainder)?;
        let (balance_value, remainder) = U512::from_bytes(remainder)?;
        let (bonded_amount_value, remainder) = U512::from_bytes(remainder)?;
        let genesis_account = GenesisAccount {
            public_key,
            account_hash,
            balance: Motes::new(balance_value),
            bonded_amount: Motes::new(bonded_amount_value),
        };
        Ok((genesis_account, remainder))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisConfig {
    name: String,
    timestamp: u64,
    protocol_version: ProtocolVersion,
    ee_config: ExecConfig,
}

impl GenesisConfig {
    pub fn new(
        name: String,
        timestamp: u64,
        protocol_version: ProtocolVersion,
        ee_config: ExecConfig,
    ) -> Self {
        GenesisConfig {
            name,
            timestamp,
            protocol_version,
            ee_config,
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    pub fn ee_config(&self) -> &ExecConfig {
        &self.ee_config
    }

    pub fn ee_config_mut(&mut self) -> &mut ExecConfig {
        &mut self.ee_config
    }

    pub fn take_ee_config(self) -> ExecConfig {
        self.ee_config
    }
}

impl Distribution<GenesisConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisConfig {
        let count = rng.gen_range(1, 1000);
        let name = iter::repeat(())
            .map(|_| rng.gen::<char>())
            .take(count)
            .collect();

        let timestamp = rng.gen();

        let protocol_version = ProtocolVersion::from_parts(rng.gen(), rng.gen(), rng.gen());

        let ee_config = rng.gen();

        GenesisConfig {
            name,
            timestamp,
            protocol_version,
            ee_config,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecConfig {
    accounts: Vec<GenesisAccount>,
    wasm_config: WasmConfig,
    validator_slots: u32,
    auction_delay: u64,
    locked_funds_period: EraId,
    round_seigniorage_rate: Ratio<u64>,
    unbonding_delay: EraId,
    wasmless_transfer_cost: u64,
}

impl ExecConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        accounts: Vec<GenesisAccount>,
        wasm_config: WasmConfig,
        validator_slots: u32,
        auction_delay: u64,
        locked_funds_period: EraId,
        round_seigniorage_rate: Ratio<u64>,
        unbonding_delay: EraId,
        wasmless_transfer_cost: u64,
    ) -> ExecConfig {
        ExecConfig {
            accounts,
            wasm_config,
            validator_slots,
            auction_delay,
            locked_funds_period,
            round_seigniorage_rate,
            unbonding_delay,
            wasmless_transfer_cost,
        }
    }

    pub fn wasm_config(&self) -> &WasmConfig {
        &self.wasm_config
    }

    pub fn get_bonded_validators(&self) -> impl Iterator<Item = &GenesisAccount> {
        self.accounts
            .iter()
            .filter(|&genesis_account| !genesis_account.bonded_amount().is_zero())
    }

    pub fn accounts(&self) -> &[GenesisAccount] {
        self.accounts.as_slice()
    }

    pub fn push_account(&mut self, account: GenesisAccount) {
        self.accounts.push(account)
    }

    pub fn validator_slots(&self) -> u32 {
        self.validator_slots
    }

    pub fn auction_delay(&self) -> u64 {
        self.auction_delay
    }

    pub fn locked_funds_period(&self) -> EraId {
        self.locked_funds_period
    }

    pub fn round_seigniorage_rate(&self) -> Ratio<u64> {
        self.round_seigniorage_rate
    }

    pub fn unbonding_delay(&self) -> EraId {
        self.unbonding_delay
    }

    pub fn wasmless_transfer_cost(&self) -> u64 {
        self.wasmless_transfer_cost
    }
}

impl Distribution<ExecConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ExecConfig {
        let count = rng.gen_range(1, 10);

        let accounts = iter::repeat(()).map(|_| rng.gen()).take(count).collect();

        let wasm_config = rng.gen();

        let validator_slots = rng.gen();

        let auction_delay = rng.gen();

        let locked_funds_period: EraId = rng.gen();

        let unbonding_delay = rng.gen();

        let round_seigniorage_rate = Ratio::new(
            rng.gen_range(1, 1_000_000_000),
            rng.gen_range(1, 1_000_000_000),
        );
        let wasmless_transfer_cost = rng.gen();

        ExecConfig {
            accounts,
            wasm_config,
            validator_slots,
            auction_delay,
            locked_funds_period,
            round_seigniorage_rate,
            unbonding_delay,
            wasmless_transfer_cost,
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub(crate) enum GenesisPurse {
    ProofOfStake {
        purse_uref: URef,
    },
    DelegatorReward {
        purse_uref: URef,
    },
    ValidatorReward {
        purse_uref: URef,
    },
    GenesisAccount {
        purse_uref: URef,
        account_hash: AccountHash,
    },
    GenesisValidator {
        purse_uref: URef,
        public_key: PublicKey,
        amount: U512,
    },
}

#[derive(Clone, Error, Debug)]
pub enum GenesisError {
    #[error("Create runtime failed.")]
    CreateRuntimeFailed,
    #[error("Unable to read contract.")]
    UnableToReadContract,
    #[error("Unable to create purse.")]
    UnableToCreatePurse,
    #[error("Unable to generate deploy hash.")]
    UnableToGenerateDeployHash,
    #[error("Unable to create system module.")]
    UnableToCreateSystemModule,
    #[error("Execution result error: {0}.")]
    ExecutionResultError(String),
    #[error("Missing proof of stake payment purse.")]
    MissingProofOfStakePaymentPurse,
    #[error("Missing validator reward purse.")]
    MissingValidatorRewardPurse,
    #[error("Missing delegator reward purse.")]
    MissingDelegatorRewardPurse,
    #[error("Mint error: {0}.")]
    MintError(mint::Error),
    #[error("CLValue error: {0}.")]
    CLValue(String),
}

pub(crate) struct GenesisInstaller<S>
where
    S: StateProvider,
    S::Error: Into<execution::Error>,
{
    correlation_id: CorrelationId,
    protocol_version: ProtocolVersion,
    exec_config: ExecConfig,
    address_generators: Rc<RefCell<AddressGenerators>>,
    virtual_system_account: Account,
    executor: Executor,
    tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
}

impl<S> GenesisInstaller<S>
where
    S: StateProvider,
    S::Error: Into<execution::Error>,
{
    pub(crate) fn new(
        correlation_id: CorrelationId,
        genesis_config_hash: Blake2bHash,
        protocol_version: ProtocolVersion,
        engine_config: EngineConfig,
        exec_config: ExecConfig,
        tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
    ) -> Self {
        let virtual_system_account = {
            let named_keys = NamedKeys::new();
            let purse = URef::new(Default::default(), AccessRights::READ_ADD_WRITE);
            Account::create(SYSTEM_ACCOUNT_ADDR, named_keys, purse)
        };

        let key = Key::Account(SYSTEM_ACCOUNT_ADDR);
        let value = { StoredValue::Account(virtual_system_account.clone()) };

        tracking_copy.borrow_mut().write(key, value);

        let executor = Executor::new(engine_config);

        let phase = Phase::System;

        let address_generators = {
            let deploy_hash = {
                let bytes: Vec<u8> = genesis_config_hash.to_vec();
                DeployHash::new(Blake2bHash::new(&bytes).value())
            };
            let generator = AddressGenerators::new(&deploy_hash, phase);
            Rc::new(RefCell::new(generator))
        };

        GenesisInstaller {
            correlation_id,
            protocol_version,
            exec_config,
            address_generators,
            virtual_system_account,
            executor,
            tracking_copy,
        }
    }

    pub(crate) fn into_execution_effect(self) -> ExecutionEffect {
        self.tracking_copy.borrow_mut().effect()
    }

    pub(crate) fn create_mint(&self) -> Result<(ContractHash, Vec<GenesisPurse>), GenesisError> {
        let access_key = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let round_seigniorage_rate_uref =
            {
                let round_seigniorage_rate_uref = self
                    .address_generators
                    .borrow_mut()
                    .new_uref(AccessRights::READ_ADD_WRITE);

                let (round_seigniorage_rate_numer, round_seigniorage_rate_denom) =
                    self.exec_config.round_seigniorage_rate().into();

                let round_seigniorage_rate: Ratio<U512> = Ratio::new(
                    round_seigniorage_rate_numer.into(),
                    round_seigniorage_rate_denom.into(),
                );

                self.tracking_copy.borrow_mut().write(
                    round_seigniorage_rate_uref.into(),
                    StoredValue::CLValue(CLValue::from_t(round_seigniorage_rate).map_err(
                        |_| GenesisError::CLValue(ARG_ROUND_SEIGNIORAGE_RATE.to_string()),
                    )?),
                );
                round_seigniorage_rate_uref
            };

        let total_supply_uref = {
            let total_supply_uref = self
                .address_generators
                .borrow_mut()
                .new_uref(AccessRights::READ_ADD_WRITE);

            self.tracking_copy.borrow_mut().write(
                total_supply_uref.into(),
                StoredValue::CLValue(
                    CLValue::from_t(U512::zero())
                        .map_err(|_| GenesisError::CLValue(TOTAL_SUPPLY_KEY.to_string()))?,
                ),
            );
            total_supply_uref
        };

        let named_keys = {
            let mut named_keys = NamedKeys::new();
            named_keys.insert(
                ROUND_SEIGNIORAGE_RATE_KEY.to_string(),
                round_seigniorage_rate_uref.into(),
            );
            named_keys.insert(TOTAL_SUPPLY_KEY.to_string(), total_supply_uref.into());
            named_keys
        };

        let entry_points = self.mint_entry_points();

        let (_, mint_hash) = self.store_contract(access_key, named_keys, entry_points);

        let purses: Vec<GenesisPurse> = self.create_purses(mint_hash)?;

        Ok((mint_hash, purses))
    }

    pub(crate) fn create_proof_of_stake(
        &self,
        genesis_purses: &[GenesisPurse],
    ) -> Result<ContractHash, GenesisError> {
        let proof_of_stake_payment_purse = genesis_purses
            .iter()
            .find_map(|item| match item {
                GenesisPurse::ProofOfStake { purse_uref, .. } => Some(purse_uref),
                _ => None,
            })
            .ok_or(GenesisError::MissingProofOfStakePaymentPurse)?;

        let named_keys = {
            let mut named_keys = NamedKeys::new();
            let named_key = Key::URef(*proof_of_stake_payment_purse);
            named_keys.insert(POS_PAYMENT_PURSE.to_string(), named_key);
            named_keys
        };

        let entry_points = self.proof_of_stake_entry_points();

        let access_key = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, proof_of_stake_hash) = self.store_contract(access_key, named_keys, entry_points);

        Ok(proof_of_stake_hash)
    }

    pub(crate) fn create_auction(
        &self,
        genesis_purses: &[GenesisPurse],
    ) -> Result<ContractHash, GenesisError> {
        // configured values
        let locked_funds_period = self.exec_config.locked_funds_period();
        let auction_delay: u64 = self.exec_config.auction_delay();

        let mut named_keys = NamedKeys::new();
        let mut validators = Bids::new();

        let validator_reward_purse = genesis_purses
            .iter()
            .find_map(|item| match item {
                GenesisPurse::ValidatorReward { purse_uref, .. } => Some(purse_uref),
                _ => None,
            })
            .ok_or(GenesisError::MissingValidatorRewardPurse)?;
        let named_key = Key::URef(*validator_reward_purse);
        named_keys.insert(VALIDATOR_REWARD_PURSE_KEY.into(), named_key);

        let delegator_reward_purse = genesis_purses
            .iter()
            .find_map(|item| match item {
                GenesisPurse::DelegatorReward { purse_uref, .. } => Some(purse_uref),
                _ => None,
            })
            .ok_or(GenesisError::MissingDelegatorRewardPurse)?;
        let named_key = Key::URef(*delegator_reward_purse);
        named_keys.insert(DELEGATOR_REWARD_PURSE_KEY.into(), named_key);

        for purses in genesis_purses {
            match purses {
                GenesisPurse::GenesisValidator {
                    public_key,
                    purse_uref,
                    amount,
                    ..
                } => {
                    let founding_validator = Bid::locked(*purse_uref, *amount, locked_funds_period);
                    validators.insert(*public_key, founding_validator);
                }
                _ => continue,
            }
        }

        let initial_seigniorage_recipients =
            self.initial_seigniorage_recipients(&validators, auction_delay);

        let era_id_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            era_id_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(INITIAL_ERA_ID)
                    .map_err(|_| GenesisError::CLValue(ERA_ID_KEY.to_string()))?,
            ),
        );
        named_keys.insert(ERA_ID_KEY.into(), era_id_uref.into());

        let initial_seigniorage_recipients_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            initial_seigniorage_recipients_uref.into(),
            StoredValue::CLValue(CLValue::from_t(initial_seigniorage_recipients).map_err(
                |_| GenesisError::CLValue(SEIGNIORAGE_RECIPIENTS_SNAPSHOT_KEY.to_string()),
            )?),
        );
        named_keys.insert(
            SEIGNIORAGE_RECIPIENTS_SNAPSHOT_KEY.into(),
            initial_seigniorage_recipients_uref.into(),
        );

        let bids_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            bids_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(validators)
                    .map_err(|_| GenesisError::CLValue(BIDS_KEY.to_string()))?,
            ),
        );
        named_keys.insert(BIDS_KEY.into(), bids_uref.into());

        let unbonding_purses_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            unbonding_purses_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(UnbondingPurses::new())
                    .map_err(|_| GenesisError::CLValue(UNBONDING_PURSES_KEY.to_string()))?,
            ),
        );
        named_keys.insert(UNBONDING_PURSES_KEY.into(), unbonding_purses_uref.into());

        let validator_slots = self.exec_config.validator_slots();
        let validator_slots_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            validator_slots_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(validator_slots)
                    .map_err(|_| GenesisError::CLValue(VALIDATOR_SLOTS_KEY.to_string()))?,
            ),
        );
        named_keys.insert(VALIDATOR_SLOTS_KEY.into(), validator_slots_uref.into());

        let auction_delay_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            auction_delay_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(auction_delay)
                    .map_err(|_| GenesisError::CLValue(AUCTION_DELAY_KEY.to_string()))?,
            ),
        );
        named_keys.insert(AUCTION_DELAY_KEY.into(), auction_delay_uref.into());

        let locked_funds_period_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            locked_funds_period_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(locked_funds_period)
                    .map_err(|_| GenesisError::CLValue(LOCKED_FUNDS_PERIOD_KEY.to_string()))?,
            ),
        );
        named_keys.insert(
            LOCKED_FUNDS_PERIOD_KEY.into(),
            locked_funds_period_uref.into(),
        );

        let unbonding_delay = self.exec_config.unbonding_delay();
        let unbonding_delay_uref = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            unbonding_delay_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(unbonding_delay)
                    .map_err(|_| GenesisError::CLValue(UNBONDING_DELAY_KEY.to_string()))?,
            ),
        );
        named_keys.insert(UNBONDING_DELAY_KEY.into(), unbonding_delay_uref.into());

        let entry_points = self.auction_entry_points();

        let access_key = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, auction_hash) = self.store_contract(access_key, named_keys, entry_points);

        Ok(auction_hash)
    }

    pub(crate) fn create_standard_payment(&self) -> ContractHash {
        let named_keys = NamedKeys::new();

        let entry_points = self.standard_payment_entry_points();

        let access_key = self
            .address_generators
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, standard_payment_hash) = self.store_contract(access_key, named_keys, entry_points);

        standard_payment_hash
    }

    pub(crate) fn create_accounts(&self, genesis_purses: &[GenesisPurse]) {
        for purse in genesis_purses {
            match purse {
                GenesisPurse::GenesisAccount {
                    purse_uref,
                    account_hash,
                    ..
                } => {
                    let account_key = Key::Account(*account_hash);
                    let account = {
                        let main_purse = *purse_uref;
                        StoredValue::Account(Account::create(
                            *account_hash,
                            NamedKeys::new(),
                            main_purse,
                        ))
                    };
                    self.tracking_copy.borrow_mut().write(account_key, account)
                }
                _ => continue,
            }
        }
    }

    fn mint_entry_points(&self) -> EntryPoints {
        let mut entry_points = EntryPoints::new();

        let entry_point = EntryPoint::new(
            METHOD_MINT,
            vec![Parameter::new(ARG_AMOUNT, CLType::U512)],
            CLType::Result {
                ok: Box::new(CLType::URef),
                err: Box::new(CLType::U8),
            },
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_REDUCE_TOTAL_SUPPLY,
            vec![Parameter::new(ARG_AMOUNT, CLType::U512)],
            CLType::Result {
                ok: Box::new(CLType::Unit),
                err: Box::new(CLType::U8),
            },
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_CREATE,
            Parameters::new(),
            CLType::URef,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_BALANCE,
            vec![Parameter::new(ARG_PURSE, CLType::URef)],
            CLType::Option(Box::new(CLType::U512)),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_TRANSFER,
            vec![
                Parameter::new(ARG_SOURCE, CLType::URef),
                Parameter::new(ARG_TARGET, CLType::URef),
                Parameter::new(ARG_AMOUNT, CLType::U512),
                Parameter::new(ARG_ID, CLType::Option(Box::new(CLType::U64))),
            ],
            CLType::Result {
                ok: Box::new(CLType::Unit),
                err: Box::new(CLType::U8),
            },
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_READ_BASE_ROUND_REWARD,
            Parameters::new(),
            CLType::U512,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        entry_points
    }

    fn proof_of_stake_entry_points(&self) -> EntryPoints {
        let mut entry_points = EntryPoints::new();

        let get_payment_purse = EntryPoint::new(
            METHOD_GET_PAYMENT_PURSE,
            vec![],
            CLType::URef,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(get_payment_purse);

        let set_refund_purse = EntryPoint::new(
            METHOD_SET_REFUND_PURSE,
            vec![Parameter::new(ARG_PURSE, CLType::URef)],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(set_refund_purse);

        let get_refund_purse = EntryPoint::new(
            METHOD_GET_REFUND_PURSE,
            vec![],
            CLType::Option(Box::new(CLType::URef)),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(get_refund_purse);

        let finalize_payment = EntryPoint::new(
            METHOD_FINALIZE_PAYMENT,
            vec![
                Parameter::new(ARG_AMOUNT, CLType::U512),
                Parameter::new(ARG_ACCOUNT, CLType::ByteArray(32)),
            ],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(finalize_payment);

        entry_points
    }

    fn auction_entry_points(&self) -> EntryPoints {
        let mut entry_points = EntryPoints::new();

        let entry_point = EntryPoint::new(
            METHOD_GET_ERA_VALIDATORS,
            vec![],
            Option::<ValidatorWeights>::cl_type(),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_READ_SEIGNIORAGE_RECIPIENTS,
            vec![],
            SeigniorageRecipients::cl_type(),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_ADD_BID,
            vec![
                Parameter::new(ARG_PUBLIC_KEY, AccountHash::cl_type()),
                Parameter::new(ARG_SOURCE_PURSE, URef::cl_type()),
                Parameter::new(ARG_DELEGATION_RATE, DelegationRate::cl_type()),
                Parameter::new(ARG_AMOUNT, U512::cl_type()),
            ],
            U512::cl_type(),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_WITHDRAW_BID,
            vec![
                Parameter::new(ARG_PUBLIC_KEY, AccountHash::cl_type()),
                Parameter::new(ARG_AMOUNT, U512::cl_type()),
                Parameter::new(ARG_UNBOND_PURSE, URef::cl_type()),
            ],
            U512::cl_type(),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_DELEGATE,
            vec![
                Parameter::new(ARG_DELEGATOR, PublicKey::cl_type()),
                Parameter::new(ARG_SOURCE_PURSE, URef::cl_type()),
                Parameter::new(ARG_VALIDATOR, PublicKey::cl_type()),
                Parameter::new(ARG_AMOUNT, U512::cl_type()),
            ],
            U512::cl_type(),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_UNDELEGATE,
            vec![
                Parameter::new(ARG_DELEGATOR, AccountHash::cl_type()),
                Parameter::new(ARG_VALIDATOR, AccountHash::cl_type()),
                Parameter::new(ARG_AMOUNT, U512::cl_type()),
                Parameter::new(ARG_UNBOND_PURSE, URef::cl_type()),
            ],
            U512::cl_type(),
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_RUN_AUCTION,
            vec![],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_SLASH,
            vec![],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_DISTRIBUTE,
            vec![Parameter::new(
                ARG_REWARD_FACTORS,
                CLType::Map {
                    key: Box::new(CLType::PublicKey),
                    value: Box::new(CLType::U64),
                },
            )],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_WITHDRAW_DELEGATOR_REWARD,
            vec![
                Parameter::new(ARG_VALIDATOR_PUBLIC_KEY, CLType::PublicKey),
                Parameter::new(ARG_DELEGATOR_PUBLIC_KEY, CLType::PublicKey),
                Parameter::new(ARG_TARGET_PURSE, CLType::URef),
            ],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_WITHDRAW_VALIDATOR_REWARD,
            vec![
                Parameter::new(ARG_VALIDATOR_PUBLIC_KEY, CLType::PublicKey),
                Parameter::new(ARG_TARGET_PURSE, CLType::URef),
            ],
            CLType::Unit,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        let entry_point = EntryPoint::new(
            METHOD_READ_ERA_ID,
            vec![],
            CLType::U64,
            EntryPointAccess::Public,
            EntryPointType::Contract,
        );
        entry_points.add_entry_point(entry_point);

        entry_points
    }

    fn standard_payment_entry_points(&self) -> EntryPoints {
        let mut entry_points = EntryPoints::new();

        let entry_point = EntryPoint::new(
            METHOD_CALL.to_string(),
            vec![Parameter::new(ARG_AMOUNT, CLType::U512)],
            CLType::Result {
                ok: Box::new(CLType::Unit),
                err: Box::new(CLType::U32),
            },
            EntryPointAccess::Public,
            EntryPointType::Session,
        );
        entry_points.add_entry_point(entry_point);

        entry_points
    }

    fn initial_seigniorage_recipients(
        &self,
        validators: &BTreeMap<PublicKey, Bid>,
        auction_delay: u64,
    ) -> BTreeMap<u64, SeigniorageRecipients> {
        let initial_snapshot_range = INITIAL_ERA_ID..=INITIAL_ERA_ID + auction_delay;

        let mut seigniorage_recipients = SeigniorageRecipients::new();
        for (era_validator, founding_validator) in validators {
            seigniorage_recipients.insert(
                *era_validator,
                SeigniorageRecipient::from(founding_validator),
            );
        }

        let mut initial_seigniorage_recipients = SeigniorageRecipientsSnapshot::new();
        for era_id in initial_snapshot_range {
            initial_seigniorage_recipients.insert(era_id, seigniorage_recipients.clone());
        }
        initial_seigniorage_recipients
    }

    fn create_purses(&self, mint_hash: ContractHash) -> Result<Vec<GenesisPurse>, GenesisError> {
        let protocol_data = ProtocolData::partial_with_mint(mint_hash);

        let preprocessor = {
            let wasm_config = protocol_data.wasm_config();
            Preprocessor::new(*wasm_config)
        };

        let system_module = self
            .tracking_copy
            .borrow_mut()
            .get_system_module(&preprocessor)
            .map_err(|_| GenesisError::UnableToCreateSystemModule)?;

        let mut purses = vec![];

        let zero = U512::zero();

        let uref = self.create_purse(zero, protocol_data, system_module.clone())?;
        purses.push(GenesisPurse::ProofOfStake { purse_uref: uref });
        let uref = self.create_purse(zero, protocol_data, system_module.clone())?;
        purses.push(GenesisPurse::DelegatorReward { purse_uref: uref });
        let uref = self.create_purse(zero, protocol_data, system_module.clone())?;
        purses.push(GenesisPurse::ValidatorReward { purse_uref: uref });

        let genesis_validators: BTreeMap<PublicKey, U512> = self
            .exec_config
            .accounts()
            .iter()
            .filter_map(|genesis_account| {
                if genesis_account.is_genesis_validator() {
                    // NOTE: Safe as genesis validators are expected to have public key
                    // specified.
                    Some((
                        genesis_account
                            .public_key()
                            .expect("should have public key"),
                        genesis_account.bonded_amount().value(),
                    ))
                } else {
                    None
                }
            })
            .collect();

        for (public_key, amount) in genesis_validators {
            let uref = self.create_purse(amount, protocol_data, system_module.clone())?;
            let genesis_validator = GenesisPurse::GenesisValidator {
                purse_uref: uref,
                public_key,
                amount,
            };
            purses.push(genesis_validator);
        }

        let accounts = {
            let mut ret: Vec<GenesisAccount> = self.exec_config.accounts().to_vec();
            let system_account = GenesisAccount::system(Motes::zero(), Motes::zero());
            ret.push(system_account);
            ret
        };

        for account in accounts {
            let amount = account.balance.value();
            let uref = self.create_purse(amount, protocol_data, system_module.clone())?;
            let genesis_account = GenesisPurse::GenesisAccount {
                purse_uref: uref,
                account_hash: account.account_hash,
            };
            purses.push(genesis_account);
        }

        Ok(purses)
    }

    fn create_purse(
        &self,
        amount: U512,
        protocol_data: ProtocolData,
        system_module: Module,
    ) -> Result<URef, GenesisError> {
        let base_key = Key::from(protocol_data.mint());

        let contract = {
            if let StoredValue::Contract(contract) = self
                .tracking_copy
                .borrow_mut()
                .read(self.correlation_id, &base_key)
                .map_err(|_| GenesisError::UnableToReadContract)?
                .ok_or(GenesisError::UnableToReadContract)?
            {
                contract
            } else {
                return Err(GenesisError::UnableToReadContract);
            }
        };

        let mut named_keys = contract.named_keys().to_owned();

        let runtime_args = runtime_args! {ARG_AMOUNT => amount};

        let authorization_keys = {
            let mut ret = BTreeSet::new();
            ret.insert(SYSTEM_ACCOUNT_ADDR);
            ret
        };

        let deploy_hash = {
            // seeds address generator w/ protocol version
            let bytes: Vec<u8> = self
                .protocol_version
                .value()
                .into_bytes()
                .map_err(|_| GenesisError::UnableToGenerateDeployHash)?
                .to_vec();
            DeployHash::new(Blake2bHash::new(&bytes).value())
        };

        let (maybe_uref, execution_result): (Option<Result<URef, mint::Error>>, ExecutionResult) =
            self.executor
                .exec_system_contract::<<S as StateProvider>::Reader, Result<URef, mint::Error>>(
                    DirectSystemContractCall::Mint,
                    system_module,
                    runtime_args,
                    &mut named_keys,
                    Default::default(),
                    base_key,
                    &self.virtual_system_account,
                    authorization_keys,
                    BlockTime::default(),
                    deploy_hash,
                    Gas::new(U512::from(std::u64::MAX)),
                    self.protocol_version,
                    self.correlation_id,
                    Rc::clone(&self.tracking_copy),
                    protocol_data,
                    SystemContractCache::default(),
                    Rc::clone(&self.address_generators),
                );

        if let Some(error) = execution_result.as_error() {
            return Err(GenesisError::ExecutionResultError(error.to_string()));
        }

        let uref = maybe_uref
            .ok_or(GenesisError::UnableToCreatePurse)?
            .map_err(GenesisError::MintError)?;
        Ok(uref)
    }

    fn store_contract(
        &self,
        access_key: URef,
        named_keys: NamedKeys,
        entry_points: EntryPoints,
    ) -> (ContractPackageHash, ContractHash) {
        let protocol_version = self.protocol_version;
        let (contract_wasm_hash, contract_hash, contract_package_hash) = {
            let mut address_generators = self.address_generators.borrow_mut();
            (
                address_generators.new_hash_address(),
                address_generators.new_hash_address(),
                address_generators.new_hash_address(),
            )
        };
        let contract_wasm = ContractWasm::new(vec![]);
        let contract = Contract::new(
            contract_package_hash,
            contract_wasm_hash,
            named_keys,
            entry_points,
            protocol_version,
        );
        let contract_package = {
            let mut contract_package = ContractPackage::new(
                access_key,
                ContractVersions::default(),
                DisabledVersions::default(),
                Groups::default(),
            );
            contract_package.insert_contract_version(protocol_version.value().major, contract_hash);
            contract_package
        };

        self.tracking_copy.borrow_mut().write(
            contract_wasm_hash.into(),
            StoredValue::ContractWasm(contract_wasm),
        );
        self.tracking_copy
            .borrow_mut()
            .write(contract_hash.into(), StoredValue::Contract(contract));
        self.tracking_copy.borrow_mut().write(
            contract_package_hash.into(),
            StoredValue::ContractPackage(contract_package),
        );

        (contract_package_hash, contract_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytesrepr_roundtrip() {
        let mut rng = rand::thread_rng();
        let genesis_account: GenesisAccount = rng.gen();
        bytesrepr::test_serialization_roundtrip(&genesis_account);
    }
}
