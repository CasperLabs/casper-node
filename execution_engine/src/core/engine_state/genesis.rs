use std::{cell::RefCell, collections::BTreeMap, fmt, iter, rc::Rc};

use datasize::DataSize;
use num_rational::Ratio;
use num_traits::Zero;
use parity_wasm::elements::Module;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use serde::{Deserialize, Serialize};

use casper_types::{
    account::AccountHash,
    bytesrepr::{self, FromBytes, ToBytes, U8_SERIALIZED_LENGTH},
    contracts::{ContractPackageStatus, ContractVersions, DisabledVersions, Groups, NamedKeys},
    runtime_args,
    system::{
        auction::{
            self, Bid, Bids, DelegationRate, Delegator, SeigniorageRecipient,
            SeigniorageRecipients, SeigniorageRecipientsSnapshot, AUCTION_DELAY_KEY,
            DELEGATION_RATE_DENOMINATOR, ERA_END_TIMESTAMP_MILLIS_KEY, ERA_ID_KEY,
            INITIAL_ERA_END_TIMESTAMP_MILLIS, INITIAL_ERA_ID, LOCKED_FUNDS_PERIOD_KEY,
            SEIGNIORAGE_RECIPIENTS_SNAPSHOT_KEY, UNBONDING_DELAY_KEY, VALIDATOR_SLOTS_KEY,
        },
        handle_payment::{self},
        mint::{
            self, ARG_AMOUNT, ARG_ROUND_SEIGNIORAGE_RATE, METHOD_MINT, ROUND_SEIGNIORAGE_RATE_KEY,
            TOTAL_SUPPLY_KEY,
        },
        standard_payment,
    },
    AccessRights, CLValue, Contract, ContractHash, ContractPackage, ContractPackageHash,
    ContractWasm, ContractWasmHash, DeployHash, EntryPointType, EntryPoints, EraId, Key, Phase,
    ProtocolVersion, PublicKey, RuntimeArgs, SecretKey, URef, U512,
};

use crate::{
    core::{
        engine_state::{execution_effect::ExecutionEffect, EngineConfig},
        execution,
        execution::{AddressGenerator, Executor},
        tracking_copy::TrackingCopy,
    },
    shared::{
        account::Account,
        gas::Gas,
        motes::Motes,
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
        system_config::SystemConfig,
        wasm_config::WasmConfig,
        TypeMismatch,
    },
    storage::{
        global_state::{CommitResult, StateProvider},
        protocol_data::ProtocolData,
    },
};

pub const PLACEHOLDER_KEY: Key = Key::Hash([0u8; 32]);
const TAG_LENGTH: usize = U8_SERIALIZED_LENGTH;

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

#[repr(u8)]
enum GenesisAccountTag {
    System = 0,
    Account = 1,
    Delegator = 2,
}

#[derive(DataSize, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisValidator {
    bonded_amount: Motes,
    delegation_rate: DelegationRate,
}

impl ToBytes for GenesisValidator {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.bonded_amount.to_bytes()?);
        buffer.extend(self.delegation_rate.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.bonded_amount.serialized_length() + self.delegation_rate.serialized_length()
    }
}

impl FromBytes for GenesisValidator {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (bonded_amount, remainder) = FromBytes::from_bytes(bytes)?;
        let (delegation_rate, remainder) = FromBytes::from_bytes(remainder)?;
        let genesis_validator = GenesisValidator {
            bonded_amount,
            delegation_rate,
        };
        Ok((genesis_validator, remainder))
    }
}

impl GenesisValidator {
    pub fn new(bonded_amount: Motes, delegation_rate: DelegationRate) -> Self {
        Self {
            bonded_amount,
            delegation_rate,
        }
    }

    pub fn bonded_amount(&self) -> Motes {
        self.bonded_amount
    }

    pub fn delegation_rate(&self) -> DelegationRate {
        self.delegation_rate
    }
}

impl Distribution<GenesisValidator> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisValidator {
        let bonded_amount = Motes::new(rng.gen());
        let delegation_rate = rng.gen();

        GenesisValidator::new(bonded_amount, delegation_rate)
    }
}

#[derive(DataSize, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenesisAccount {
    System,
    Account {
        public_key: PublicKey,
        balance: Motes,
        validator: Option<GenesisValidator>,
    },
    Delegator {
        validator_public_key: PublicKey,
        delegator_public_key: PublicKey,
        balance: Motes,
        delegated_amount: Motes,
    },
}

impl GenesisAccount {
    /// Create a system account variant.
    pub fn system() -> Self {
        Self::System
    }

    /// Create a standard account variant.
    pub fn account(
        public_key: PublicKey,
        balance: Motes,
        validator: Option<GenesisValidator>,
    ) -> Self {
        Self::Account {
            public_key,
            balance,
            validator,
        }
    }

    /// Create a delegator account variant.
    pub fn delegator(
        validator_public_key: PublicKey,
        delegator_public_key: PublicKey,
        balance: Motes,
        delegated_amount: Motes,
    ) -> Self {
        Self::Delegator {
            validator_public_key,
            delegator_public_key,
            balance,
            delegated_amount,
        }
    }

    /// The public key (if any) associated with the account.
    pub fn public_key(&self) -> PublicKey {
        match self {
            GenesisAccount::System => PublicKey::System,
            GenesisAccount::Account { public_key, .. } => public_key.clone(),
            GenesisAccount::Delegator {
                delegator_public_key,
                ..
            } => delegator_public_key.clone(),
        }
    }

    /// The account hash for the account.
    pub fn account_hash(&self) -> AccountHash {
        match self {
            GenesisAccount::System => PublicKey::System.to_account_hash(),
            GenesisAccount::Account { public_key, .. } => public_key.to_account_hash(),
            GenesisAccount::Delegator {
                delegator_public_key,
                ..
            } => delegator_public_key.to_account_hash(),
        }
    }

    /// How many motes are to be deposited in the account's main purse.
    pub fn balance(&self) -> Motes {
        match self {
            GenesisAccount::System => Motes::zero(),
            GenesisAccount::Account { balance, .. } => *balance,
            GenesisAccount::Delegator { balance, .. } => *balance,
        }
    }

    /// How many motes are to be staked.
    ///
    /// Staked accounts are either validators with some amount of bonded stake or delgators with
    /// some amount of delegated stake.
    pub fn staked_amount(&self) -> Motes {
        match self {
            GenesisAccount::System { .. }
            | GenesisAccount::Account {
                validator: None, ..
            } => Motes::zero(),
            GenesisAccount::Account {
                validator: Some(genesis_validator),
                ..
            } => genesis_validator.bonded_amount(),
            GenesisAccount::Delegator {
                delegated_amount, ..
            } => *delegated_amount,
        }
    }

    /// What is the delegation rate of a validator.
    pub fn delegation_rate(&self) -> DelegationRate {
        match self {
            GenesisAccount::Account {
                validator: Some(genesis_validator),
                ..
            } => genesis_validator.delegation_rate(),
            GenesisAccount::System
            | GenesisAccount::Account {
                validator: None, ..
            }
            | GenesisAccount::Delegator { .. } => {
                // This value represents a delegation rate in invalid state that system is supposed
                // to reject if used.
                DelegationRate::max_value()
            }
        }
    }

    /// Is this a virtual system account.
    pub fn is_system_account(&self) -> bool {
        matches!(self, GenesisAccount::System { .. })
    }

    /// Is this a validator account.
    pub fn is_validator(&self) -> bool {
        match self {
            GenesisAccount::Account {
                validator: Some(_), ..
            } => true,
            GenesisAccount::System { .. }
            | GenesisAccount::Account {
                validator: None, ..
            }
            | GenesisAccount::Delegator { .. } => false,
        }
    }

    /// Details about the genesis validator.
    pub fn validator(&self) -> Option<&GenesisValidator> {
        match self {
            GenesisAccount::Account {
                validator: Some(genesis_validator),
                ..
            } => Some(genesis_validator),
            _ => None,
        }
    }

    /// Is this a delegator account.
    pub fn is_delegator(&self) -> bool {
        matches!(self, GenesisAccount::Delegator { .. })
    }

    /// Details about the genesis delegator.
    pub fn as_delegator(&self) -> Option<(&PublicKey, &PublicKey, &Motes, &Motes)> {
        match self {
            GenesisAccount::Delegator {
                validator_public_key,
                delegator_public_key,
                balance,
                delegated_amount,
            } => Some((
                validator_public_key,
                delegator_public_key,
                balance,
                delegated_amount,
            )),
            _ => None,
        }
    }
}

impl Distribution<GenesisAccount> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisAccount {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes[..]);
        let secret_key = SecretKey::ed25519_from_bytes(bytes).unwrap();
        let public_key = PublicKey::from(&secret_key);
        let balance = Motes::new(rng.gen());
        let validator = rng.gen();

        GenesisAccount::account(public_key, balance, validator)
    }
}

impl ToBytes for GenesisAccount {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        match self {
            GenesisAccount::System => {
                buffer.push(GenesisAccountTag::System as u8);
            }
            GenesisAccount::Account {
                public_key,
                balance,
                validator,
            } => {
                buffer.push(GenesisAccountTag::Account as u8);
                buffer.extend(public_key.to_bytes()?);
                buffer.extend(balance.value().to_bytes()?);
                buffer.extend(validator.to_bytes()?);
            }
            GenesisAccount::Delegator {
                validator_public_key,
                delegator_public_key,
                balance,
                delegated_amount,
            } => {
                buffer.push(GenesisAccountTag::Delegator as u8);
                buffer.extend(validator_public_key.to_bytes()?);
                buffer.extend(delegator_public_key.to_bytes()?);
                buffer.extend(balance.value().to_bytes()?);
                buffer.extend(delegated_amount.value().to_bytes()?);
            }
        }
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        match self {
            GenesisAccount::System => TAG_LENGTH,
            GenesisAccount::Account {
                public_key,
                balance,
                validator,
            } => {
                public_key.serialized_length()
                    + balance.value().serialized_length()
                    + validator.serialized_length()
                    + TAG_LENGTH
            }
            GenesisAccount::Delegator {
                validator_public_key,
                delegator_public_key,
                balance,
                delegated_amount,
            } => {
                validator_public_key.serialized_length()
                    + delegator_public_key.serialized_length()
                    + balance.value().serialized_length()
                    + delegated_amount.value().serialized_length()
                    + TAG_LENGTH
            }
        }
    }
}

impl FromBytes for GenesisAccount {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = u8::from_bytes(bytes)?;
        match tag {
            tag if tag == GenesisAccountTag::System as u8 => {
                let genesis_account = GenesisAccount::system();
                Ok((genesis_account, remainder))
            }
            tag if tag == GenesisAccountTag::Account as u8 => {
                let (public_key, remainder) = FromBytes::from_bytes(remainder)?;
                let (balance, remainder) = FromBytes::from_bytes(remainder)?;
                let (validator, remainder) = FromBytes::from_bytes(remainder)?;
                let genesis_account = GenesisAccount::account(public_key, balance, validator);
                Ok((genesis_account, remainder))
            }
            tag if tag == GenesisAccountTag::Delegator as u8 => {
                let (validator_public_key, remainder) = FromBytes::from_bytes(remainder)?;
                let (delegator_public_key, remainder) = FromBytes::from_bytes(remainder)?;
                let (balance, remainder) = FromBytes::from_bytes(remainder)?;
                let (delegated_amount_value, remainder) = FromBytes::from_bytes(remainder)?;
                let genesis_account = GenesisAccount::delegator(
                    validator_public_key,
                    delegator_public_key,
                    balance,
                    Motes::new(delegated_amount_value),
                );
                Ok((genesis_account, remainder))
            }
            _ => Err(bytesrepr::Error::Formatting),
        }
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
        let count = rng.gen_range(1..1000);
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecConfig {
    accounts: Vec<GenesisAccount>,
    wasm_config: WasmConfig,
    system_config: SystemConfig,
    validator_slots: u32,
    auction_delay: u64,
    locked_funds_period_millis: u64,
    round_seigniorage_rate: Ratio<u64>,
    unbonding_delay: u64,
    genesis_timestamp_millis: u64,
}

impl ExecConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        accounts: Vec<GenesisAccount>,
        wasm_config: WasmConfig,
        system_config: SystemConfig,
        validator_slots: u32,
        auction_delay: u64,
        locked_funds_period_millis: u64,
        round_seigniorage_rate: Ratio<u64>,
        unbonding_delay: u64,
        genesis_timestamp_millis: u64,
    ) -> ExecConfig {
        ExecConfig {
            accounts,
            wasm_config,
            system_config,
            validator_slots,
            auction_delay,
            locked_funds_period_millis,
            round_seigniorage_rate,
            unbonding_delay,
            genesis_timestamp_millis,
        }
    }

    pub fn wasm_config(&self) -> &WasmConfig {
        &self.wasm_config
    }

    pub fn system_config(&self) -> &SystemConfig {
        &self.system_config
    }

    pub fn get_bonded_validators(&self) -> impl Iterator<Item = &GenesisAccount> {
        self.accounts
            .iter()
            .filter(|&genesis_account| genesis_account.is_validator())
    }

    pub fn get_bonded_delegators(
        &self,
    ) -> impl Iterator<Item = (&PublicKey, &PublicKey, &Motes, &Motes)> {
        self.accounts
            .iter()
            .filter_map(|genesis_account| genesis_account.as_delegator())
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

    pub fn locked_funds_period_millis(&self) -> u64 {
        self.locked_funds_period_millis
    }

    pub fn round_seigniorage_rate(&self) -> Ratio<u64> {
        self.round_seigniorage_rate
    }

    pub fn unbonding_delay(&self) -> u64 {
        self.unbonding_delay
    }

    pub fn genesis_timestamp_millis(&self) -> u64 {
        self.genesis_timestamp_millis
    }
}

impl Distribution<ExecConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ExecConfig {
        let count = rng.gen_range(1..10);

        let accounts = iter::repeat(()).map(|_| rng.gen()).take(count).collect();

        let wasm_config = rng.gen();

        let system_config = rng.gen();

        let validator_slots = rng.gen();

        let auction_delay = rng.gen();

        let locked_funds_period_millis = rng.gen();

        let round_seigniorage_rate = Ratio::new(
            rng.gen_range(1..1_000_000_000),
            rng.gen_range(1..1_000_000_000),
        );

        let unbonding_delay = rng.gen();

        let genesis_timestamp_millis = rng.gen();

        ExecConfig {
            accounts,
            wasm_config,
            system_config,
            validator_slots,
            auction_delay,
            locked_funds_period_millis,
            round_seigniorage_rate,
            unbonding_delay,
            genesis_timestamp_millis,
        }
    }
}

#[derive(Clone, Debug)]
pub enum GenesisError {
    UnableToCreateRuntime,
    InvalidMintKey,
    MissingMintContract,
    UnexpectedStoredValue,
    MissingPublicKey,
    ExecutionError(execution::Error),
    MintError(mint::Error),
    CLValue(String),
    OrphanedDelegator {
        validator_public_key: PublicKey,
        delegator_public_key: PublicKey,
    },
    DuplicatedDelegatorEntry {
        validator_public_key: PublicKey,
        delegator_public_key: PublicKey,
    },
    InvalidDelegationRate {
        public_key: PublicKey,
        delegation_rate: DelegationRate,
    },
    InvalidBondAmount {
        public_key: PublicKey,
    },
    InvalidDelegatedAmount {
        public_key: PublicKey,
    },
    InvalidValidatorSlots {
        validators: usize,
        validator_slots: u32,
    },
}

pub(crate) struct GenesisInstaller<S>
where
    S: StateProvider,
    S::Error: Into<execution::Error>,
{
    genesis_config_hash: Blake2bHash,
    virtual_system_account: Account,
    protocol_version: ProtocolVersion,
    correlation_id: CorrelationId,
    exec_config: ExecConfig,
    uref_address_generator: Rc<RefCell<AddressGenerator>>,
    hash_address_generator: Rc<RefCell<AddressGenerator>>,
    transfer_address_generator: Rc<RefCell<AddressGenerator>>,
    executor: Executor,
    tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
    protocol_data: ProtocolData,
    system_module: Module,
}

impl<S> GenesisInstaller<S>
where
    S: StateProvider,
    S::Error: Into<execution::Error>,
{
    pub(crate) fn new(
        genesis_config_hash: Blake2bHash,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        engine_config: EngineConfig,
        exec_config: ExecConfig,
        tracking_copy: Rc<RefCell<TrackingCopy<<S as StateProvider>::Reader>>>,
        system_module: Module,
    ) -> Self {
        let executor = Executor::new(engine_config);

        let phase = Phase::System;
        let genesis_config_hash_bytes = genesis_config_hash.as_ref();
        let uref_address_generator = {
            let generator = AddressGenerator::new(genesis_config_hash_bytes, phase);
            Rc::new(RefCell::new(generator))
        };
        let hash_address_generator = {
            let generator = AddressGenerator::new(genesis_config_hash_bytes, phase);
            Rc::new(RefCell::new(generator))
        };
        let transfer_address_generator = {
            let generator = AddressGenerator::new(genesis_config_hash_bytes, phase);
            Rc::new(RefCell::new(generator))
        };

        let protocol_data = ProtocolData::default();

        let system_account_addr = PublicKey::System.to_account_hash();

        let virtual_system_account = {
            let named_keys = NamedKeys::new();
            let purse = URef::new(Default::default(), AccessRights::READ_ADD_WRITE);
            Account::create(system_account_addr, named_keys, purse)
        };

        let key = Key::Account(system_account_addr);
        let value = { StoredValue::Account(virtual_system_account.clone()) };

        tracking_copy.borrow_mut().write(key, value);

        GenesisInstaller {
            genesis_config_hash,
            virtual_system_account,
            protocol_version,
            correlation_id,
            exec_config,
            uref_address_generator,
            hash_address_generator,
            transfer_address_generator,
            executor,
            tracking_copy,
            protocol_data,
            system_module,
        }
    }

    pub(crate) fn finalize(self) -> ExecutionEffect {
        self.tracking_copy.borrow_mut().effect()
    }

    pub(crate) fn create_mint(&mut self) -> Result<ContractHash, GenesisError> {
        let round_seigniorage_rate_uref =
            {
                let round_seigniorage_rate_uref = self
                    .uref_address_generator
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
                .uref_address_generator
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

        let entry_points = mint::mint_entry_points();

        let access_key = self
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, mint_hash) = self.store_contract(access_key, named_keys, entry_points);

        self.protocol_data = ProtocolData::partial_with_mint(mint_hash);

        Ok(mint_hash)
    }

    pub fn create_handle_payment(&self) -> Result<ContractHash, GenesisError> {
        let handle_payment_payment_purse = self.create_purse(
            U512::zero(),
            DeployHash::new(self.genesis_config_hash.value()),
        )?;

        let named_keys = {
            let mut named_keys = NamedKeys::new();
            let named_key = Key::URef(handle_payment_payment_purse);
            named_keys.insert(handle_payment::PAYMENT_PURSE_KEY.to_string(), named_key);
            named_keys
        };

        let entry_points = handle_payment::handle_payment_entry_points();

        let access_key = self
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, handle_payment_hash) = self.store_contract(access_key, named_keys, entry_points);

        Ok(handle_payment_hash)
    }

    pub(crate) fn create_auction(&self) -> Result<ContractHash, GenesisError> {
        let locked_funds_period_millis = self.exec_config.locked_funds_period_millis();
        let auction_delay: u64 = self.exec_config.auction_delay();
        let genesis_timestamp_millis: u64 = self.exec_config.genesis_timestamp_millis();

        let mut named_keys = NamedKeys::new();

        let genesis_validators: Vec<_> = self.exec_config.get_bonded_validators().collect();
        if (self.exec_config.validator_slots() as usize) < genesis_validators.len() {
            return Err(GenesisError::InvalidValidatorSlots {
                validators: genesis_validators.len(),
                validator_slots: self.exec_config.validator_slots(),
            });
        }

        let genesis_delegators: Vec<_> = self.exec_config.get_bonded_delegators().collect();

        // Make sure all delegators have corresponding genesis validator entries
        for (validator_public_key, delegator_public_key, _balance, delegated_amount) in
            genesis_delegators.iter()
        {
            if delegated_amount.is_zero() {
                return Err(GenesisError::InvalidDelegatedAmount {
                    public_key: (*delegator_public_key).clone(),
                });
            }

            let orphan_condition = genesis_validators.iter().find(|genesis_validator| {
                genesis_validator.public_key() == (*validator_public_key).clone()
            });

            if orphan_condition.is_none() {
                return Err(GenesisError::OrphanedDelegator {
                    validator_public_key: (*validator_public_key).clone(),
                    delegator_public_key: (*delegator_public_key).clone(),
                });
            }
        }

        let validators = {
            let mut validators = Bids::new();

            for genesis_validator in genesis_validators {
                let public_key = genesis_validator.public_key();

                let staked_amount = genesis_validator.staked_amount();
                if staked_amount.is_zero() {
                    return Err(GenesisError::InvalidBondAmount { public_key });
                }

                let delegation_rate = genesis_validator.delegation_rate();
                if delegation_rate > DELEGATION_RATE_DENOMINATOR {
                    return Err(GenesisError::InvalidDelegationRate {
                        public_key,
                        delegation_rate,
                    });
                }
                debug_assert_ne!(public_key, PublicKey::System);

                let purse_uref = self.create_purse(
                    staked_amount.value(),
                    DeployHash::new(public_key.to_account_hash().value()),
                )?;
                let release_timestamp_millis =
                    genesis_timestamp_millis + locked_funds_period_millis;
                let founding_validator = {
                    let mut bid = Bid::locked(
                        public_key.clone(),
                        purse_uref,
                        staked_amount.value(),
                        delegation_rate,
                        release_timestamp_millis,
                    );

                    // Set up delegator entries attached to genesis validators
                    for (
                        validator_public_key,
                        delegator_public_key,
                        _delegator_balance,
                        &delegator_delegated_amount,
                    ) in genesis_delegators.iter()
                    {
                        if (*validator_public_key).clone() == public_key.clone() {
                            let purse_uref = self.create_purse(
                                delegator_delegated_amount.value(),
                                DeployHash::new(delegator_public_key.to_account_hash().value()),
                            )?;

                            let delegator = Delegator::locked(
                                (*delegator_public_key).clone(),
                                delegator_delegated_amount.value(),
                                purse_uref,
                                (*validator_public_key).clone(),
                                release_timestamp_millis,
                            );

                            if bid
                                .delegators_mut()
                                .insert((*delegator_public_key).clone(), delegator)
                                .is_some()
                            {
                                return Err(GenesisError::DuplicatedDelegatorEntry {
                                    validator_public_key: (*validator_public_key).clone(),
                                    delegator_public_key: (*delegator_public_key).clone(),
                                });
                            }
                        }
                    }

                    bid
                };

                validators.insert(public_key, founding_validator);
            }
            validators
        };

        let initial_seigniorage_recipients =
            self.initial_seigniorage_recipients(&validators, auction_delay);

        let era_id_uref = self
            .uref_address_generator
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

        let era_end_timestamp_millis_uref = self
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            era_end_timestamp_millis_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(INITIAL_ERA_END_TIMESTAMP_MILLIS)
                    .map_err(|_| GenesisError::CLValue(ERA_END_TIMESTAMP_MILLIS_KEY.to_string()))?,
            ),
        );
        named_keys.insert(
            ERA_END_TIMESTAMP_MILLIS_KEY.into(),
            era_end_timestamp_millis_uref.into(),
        );

        let initial_seigniorage_recipients_uref = self
            .uref_address_generator
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

        for (validator_public_key, bid) in validators.into_iter() {
            let validator_account_hash = AccountHash::from(&validator_public_key);
            self.tracking_copy.borrow_mut().write(
                Key::Bid(validator_account_hash),
                StoredValue::Bid(Box::new(bid)),
            )
        }

        let validator_slots = self.exec_config.validator_slots();
        let validator_slots_uref = self
            .uref_address_generator
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
            .uref_address_generator
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
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.tracking_copy.borrow_mut().write(
            locked_funds_period_uref.into(),
            StoredValue::CLValue(
                CLValue::from_t(locked_funds_period_millis)
                    .map_err(|_| GenesisError::CLValue(LOCKED_FUNDS_PERIOD_KEY.to_string()))?,
            ),
        );
        named_keys.insert(
            LOCKED_FUNDS_PERIOD_KEY.into(),
            locked_funds_period_uref.into(),
        );

        let unbonding_delay = self.exec_config.unbonding_delay();
        let unbonding_delay_uref = self
            .uref_address_generator
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

        let entry_points = auction::auction_entry_points();

        let access_key = self
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, auction_hash) = self.store_contract(access_key, named_keys, entry_points);

        Ok(auction_hash)
    }

    pub(crate) fn create_standard_payment(&self) -> ContractHash {
        let named_keys = NamedKeys::new();

        let entry_points = standard_payment::standard_payment_entry_points();

        let access_key = self
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);

        let (_, standard_payment_hash) = self.store_contract(access_key, named_keys, entry_points);

        standard_payment_hash
    }

    pub(crate) fn create_accounts(&self) -> Result<(), GenesisError> {
        let accounts = {
            let mut ret: Vec<GenesisAccount> =
                self.exec_config.accounts().to_vec().into_iter().collect();
            let system_account = GenesisAccount::system();
            ret.push(system_account);
            ret
        };

        for account in accounts {
            let account_hash = account.account_hash();
            let main_purse = self.create_purse(
                account.balance().value(),
                DeployHash::new(account_hash.value()),
            )?;

            let key = Key::Account(account_hash);
            let stored_value = StoredValue::Account(Account::create(
                account_hash,
                Default::default(),
                main_purse,
            ));

            self.tracking_copy.borrow_mut().write(key, stored_value);
        }

        Ok(())
    }

    fn initial_seigniorage_recipients(
        &self,
        validators: &BTreeMap<PublicKey, Bid>,
        auction_delay: u64,
    ) -> BTreeMap<EraId, SeigniorageRecipients> {
        let initial_snapshot_range = INITIAL_ERA_ID.iter_inclusive(auction_delay);

        let mut seigniorage_recipients = SeigniorageRecipients::new();
        for (era_validator, founding_validator) in validators {
            seigniorage_recipients.insert(
                era_validator.clone(),
                SeigniorageRecipient::from(founding_validator),
            );
        }

        let mut initial_seigniorage_recipients = SeigniorageRecipientsSnapshot::new();
        for era_id in initial_snapshot_range {
            initial_seigniorage_recipients.insert(era_id, seigniorage_recipients.clone());
        }
        initial_seigniorage_recipients
    }

    fn create_purse(&self, amount: U512, deploy_hash: DeployHash) -> Result<URef, GenesisError> {
        let args = runtime_args! {
            ARG_AMOUNT => amount,
        };

        let base_key = Key::Hash(self.protocol_data.mint().value());
        let mint = {
            if let StoredValue::Contract(contract) = self
                .tracking_copy
                .borrow_mut()
                .read(self.correlation_id, &base_key)
                .map_err(|_| GenesisError::InvalidMintKey)?
                .ok_or(GenesisError::MissingMintContract)?
            {
                contract
            } else {
                return Err(GenesisError::UnexpectedStoredValue);
            }
        };

        let mut named_keys = mint.named_keys().clone();

        let call_stack = Vec::new();

        let (_instance, mut runtime) = self
            .executor
            .create_runtime(
                self.system_module.clone(),
                EntryPointType::Contract,
                args.clone(),
                &mut named_keys,
                Default::default(),
                base_key,
                &self.virtual_system_account,
                Default::default(),
                Default::default(),
                deploy_hash,
                Gas::new(U512::MAX),
                Rc::clone(&self.hash_address_generator),
                Rc::clone(&self.uref_address_generator),
                Rc::clone(&self.transfer_address_generator),
                self.protocol_version,
                self.correlation_id,
                Rc::clone(&self.tracking_copy),
                Phase::System,
                self.protocol_data,
                Default::default(),
                call_stack,
            )
            .map_err(|_| GenesisError::UnableToCreateRuntime)?;

        let purse_uref = runtime
            .call_contract(self.protocol_data.mint(), METHOD_MINT, args)
            .map_err(GenesisError::ExecutionError)?
            .into_t::<Result<URef, mint::Error>>()
            .map_err(|cl_value_error| GenesisError::CLValue(cl_value_error.to_string()))?
            .map_err(GenesisError::MintError)?;
        Ok(purse_uref)
    }

    fn store_contract(
        &self,
        access_key: URef,
        named_keys: NamedKeys,
        entry_points: EntryPoints,
    ) -> (ContractPackageHash, ContractHash) {
        let protocol_version = self.protocol_version;
        let contract_wasm_hash =
            ContractWasmHash::new(self.hash_address_generator.borrow_mut().new_hash_address());
        let contract_hash =
            ContractHash::new(self.hash_address_generator.borrow_mut().new_hash_address());
        let contract_package_hash =
            ContractPackageHash::new(self.hash_address_generator.borrow_mut().new_hash_address());

        let contract_wasm = ContractWasm::new(vec![]);
        let contract = Contract::new(
            contract_package_hash,
            contract_wasm_hash,
            named_keys,
            entry_points,
            protocol_version,
        );

        // Genesis contracts can be versioned contracts.
        let contract_package = {
            let mut contract_package = ContractPackage::new(
                access_key,
                ContractVersions::default(),
                DisabledVersions::default(),
                Groups::default(),
                ContractPackageStatus::default(),
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
    use rand::RngCore;

    #[test]
    fn bytesrepr_roundtrip() {
        let mut rng = rand::thread_rng();
        let genesis_account: GenesisAccount = rng.gen();
        bytesrepr::test_serialization_roundtrip(&genesis_account);
    }

    #[test]
    fn system_account_bytesrepr_roundtrip() {
        let genesis_account = GenesisAccount::system();

        bytesrepr::test_serialization_roundtrip(&genesis_account);
    }

    #[test]
    fn account_bytesrepr_roundtrip() {
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes[..]);
        let secret_key = SecretKey::ed25519_from_bytes(bytes).unwrap();
        let public_key: PublicKey = PublicKey::from(&secret_key);

        let genesis_account_1 =
            GenesisAccount::account(public_key.clone(), Motes::new(U512::from(100)), None);

        bytesrepr::test_serialization_roundtrip(&genesis_account_1);

        let genesis_account_2 =
            GenesisAccount::account(public_key, Motes::new(U512::from(100)), Some(rng.gen()));

        bytesrepr::test_serialization_roundtrip(&genesis_account_2);
    }

    #[test]
    fn delegator_bytesrepr_roundtrip() {
        let mut rng = rand::thread_rng();
        let mut validator_bytes = [0u8; 32];
        let mut delegator_bytes = [0u8; 32];
        rng.fill_bytes(&mut validator_bytes[..]);
        rng.fill_bytes(&mut delegator_bytes[..]);
        let validator_secret_key = SecretKey::ed25519_from_bytes(validator_bytes).unwrap();
        let delegator_secret_key = SecretKey::ed25519_from_bytes(delegator_bytes).unwrap();

        let validator_public_key = PublicKey::from(&validator_secret_key);
        let delegator_public_key = PublicKey::from(&delegator_secret_key);

        let genesis_account = GenesisAccount::delegator(
            validator_public_key,
            delegator_public_key,
            Motes::new(U512::from(100)),
            Motes::zero(),
        );

        bytesrepr::test_serialization_roundtrip(&genesis_account);
    }
}
