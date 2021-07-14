use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    convert::{TryFrom, TryInto},
    fmt::Debug,
    rc::Rc,
};

use casper_types::{
    account::{
        AccountHash, ActionType, AddKeyFailure, RemoveKeyFailure, SetThresholdFailure,
        UpdateKeyFailure, Weight,
    },
    bytesrepr,
    bytesrepr::ToBytes,
    contracts::NamedKeys,
    system::auction::EraInfo,
    AccessRights, BlockTime, CLType, CLValue, Contract, ContractPackage, ContractPackageHash,
    DeployHash, DeployInfo, EntryPointAccess, EntryPointType, Key, KeyTag, Phase, ProtocolVersion,
    PublicKey, RuntimeArgs, Transfer, TransferAddr, URef, KEY_HASH_LENGTH,
};

use crate::{
    core::{
        engine_state::execution_effect::ExecutionEffect,
        execution::{AddressGenerator, Error},
        runtime_context::dictionary::DictionaryValue,
        tracking_copy::{AddResult, TrackingCopy},
        Address,
    },
    shared::{
        account::Account, gas::Gas, gas_counter::GasCounter, newtypes::CorrelationId,
        stored_value::StoredValue,
    },
    storage::{global_state::StateReader, protocol_data::ProtocolData},
};

pub(crate) mod dictionary;
pub mod mocks;
#[cfg(test)]
mod tests;

/// Checks whether given uref has enough access rights.
pub(crate) fn uref_has_access_rights(
    uref: &URef,
    access_rights: &HashMap<Address, HashSet<AccessRights>>,
) -> bool {
    if let Some(known_rights) = access_rights.get(&uref.addr()) {
        let new_rights = uref.access_rights();
        // check if we have sufficient access rights
        known_rights
            .iter()
            .any(|right| *right & new_rights == new_rights)
    } else {
        // URef is not known
        false
    }
}

pub fn validate_entry_point_access_with(
    contract_package: &ContractPackage,
    access: &EntryPointAccess,
    validator: impl Fn(&URef) -> bool,
) -> Result<(), Error> {
    if let EntryPointAccess::Groups(groups) = access {
        if groups.is_empty() {
            // Exits early in a special case of empty list of groups regardless of the group
            // checking logic below it.
            return Err(Error::InvalidContext);
        }

        let find_result = groups.iter().find(|g| {
            contract_package
                .groups()
                .get(g)
                .and_then(|set| set.iter().find(|u| validator(u)))
                .is_some()
        });

        if find_result.is_none() {
            return Err(Error::InvalidContext);
        }
    }
    Ok(())
}

/// Holds information specific to the deployed contract.
pub struct RuntimeContext<'a, R> {
    tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
    // Enables look up of specific uref based on human-readable name
    named_keys: &'a mut NamedKeys,
    // Used to check uref is known before use (prevents forging urefs)
    access_rights: HashMap<Address, HashSet<AccessRights>>,
    // Original account for read only tasks taken before execution
    account: &'a Account,
    args: RuntimeArgs,
    authorization_keys: BTreeSet<AccountHash>,
    // Key pointing to the entity we are currently running
    //(could point at an account or contract in the global state)
    base_key: Key,
    blocktime: BlockTime,
    deploy_hash: DeployHash,
    gas_counter: GasCounter,
    hash_address_generator: Rc<RefCell<AddressGenerator>>,
    uref_address_generator: Rc<RefCell<AddressGenerator>>,
    transfer_address_generator: Rc<RefCell<AddressGenerator>>,
    protocol_version: ProtocolVersion,
    correlation_id: CorrelationId,
    phase: Phase,
    protocol_data: ProtocolData,
    entry_point_type: EntryPointType,
    transfers: Vec<TransferAddr>,
}

impl<'a, R> RuntimeContext<'a, R>
where
    R: StateReader<Key, StoredValue>,
    R::Error: Into<Error>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tracking_copy: Rc<RefCell<TrackingCopy<R>>>,
        entry_point_type: EntryPointType,
        named_keys: &'a mut NamedKeys,
        access_rights: HashMap<Address, HashSet<AccessRights>>,
        runtime_args: RuntimeArgs,
        authorization_keys: BTreeSet<AccountHash>,
        account: &'a Account,
        base_key: Key,
        blocktime: BlockTime,
        deploy_hash: DeployHash,
        gas_counter: GasCounter,
        hash_address_generator: Rc<RefCell<AddressGenerator>>,
        uref_address_generator: Rc<RefCell<AddressGenerator>>,
        transfer_address_generator: Rc<RefCell<AddressGenerator>>,
        protocol_version: ProtocolVersion,
        correlation_id: CorrelationId,
        phase: Phase,
        protocol_data: ProtocolData,
        transfers: Vec<TransferAddr>,
    ) -> Self {
        RuntimeContext {
            tracking_copy,
            entry_point_type,
            named_keys,
            access_rights,
            args: runtime_args,
            account,
            authorization_keys,
            blocktime,
            deploy_hash,
            base_key,
            gas_counter,
            hash_address_generator,
            uref_address_generator,
            transfer_address_generator,
            protocol_version,
            correlation_id,
            phase,
            protocol_data,
            transfers,
        }
    }

    pub fn authorization_keys(&self) -> &BTreeSet<AccountHash> {
        &self.authorization_keys
    }

    pub fn named_keys_get(&self, name: &str) -> Option<&Key> {
        self.named_keys.get(name)
    }

    pub fn named_keys(&self) -> &NamedKeys {
        self.named_keys
    }

    pub fn named_keys_mut(&mut self) -> &mut NamedKeys {
        &mut self.named_keys
    }

    pub fn named_keys_contains_key(&self, name: &str) -> bool {
        self.named_keys.contains_key(name)
    }

    // Helper function to avoid duplication in `remove_uref`.
    fn remove_key_from_contract(
        &mut self,
        key: Key,
        mut contract: Contract,
        name: &str,
    ) -> Result<(), Error> {
        if contract.remove_named_key(name).is_none() {
            return Ok(());
        }
        self.metered_write_gs_unsafe(key, contract)?;
        Ok(())
    }

    /// Remove Key from the `named_keys` map of the current context.
    /// It removes both from the ephemeral map (RuntimeContext::named_keys) but
    /// also persistable map (one that is found in the
    /// TrackingCopy/GlobalState).
    pub fn remove_key(&mut self, name: &str) -> Result<(), Error> {
        match self.base_key() {
            account_hash @ Key::Account(_) => {
                let account: Account = {
                    let mut account: Account = self.read_gs_typed(&account_hash)?;
                    account.named_keys_mut().remove(name);
                    account
                };
                self.named_keys.remove(name);
                let account_value = self.account_to_validated_value(account)?;
                self.metered_write_gs_unsafe(account_hash, account_value)?;
                Ok(())
            }
            contract_uref @ Key::URef(_) => {
                let contract: Contract = {
                    let value: StoredValue = self
                        .tracking_copy
                        .borrow_mut()
                        .read(self.correlation_id, &contract_uref)
                        .map_err(Into::into)?
                        .ok_or(Error::KeyNotFound(contract_uref))?;

                    value.try_into().map_err(Error::TypeMismatch)?
                };

                self.named_keys.remove(name);
                self.remove_key_from_contract(contract_uref, contract, name)
            }
            contract_hash @ Key::Hash(_) => {
                let contract: Contract = self.read_gs_typed(&contract_hash)?;
                self.named_keys.remove(name);
                self.remove_key_from_contract(contract_hash, contract, name)
            }
            transfer_addr @ Key::Transfer(_) => {
                let _transfer: Transfer = self.read_gs_typed(&transfer_addr)?;
                self.named_keys.remove(name);
                // Users cannot remove transfers from global state
                Ok(())
            }
            deploy_info_addr @ Key::DeployInfo(_) => {
                let _deploy_info: DeployInfo = self.read_gs_typed(&deploy_info_addr)?;
                self.named_keys.remove(name);
                // Users cannot remove deploy infos from global state
                Ok(())
            }
            era_info_addr @ Key::EraInfo(_) => {
                let _era_info: EraInfo = self.read_gs_typed(&era_info_addr)?;
                self.named_keys.remove(name);
                // Users cannot remove era infos from global state
                Ok(())
            }
            Key::Balance(_) => {
                self.named_keys.remove(name);
                Ok(())
            }
            Key::Bid(_) => {
                self.named_keys.remove(name);
                Ok(())
            }
            Key::Withdraw(_) => {
                self.named_keys.remove(name);
                Ok(())
            }
            Key::Dictionary(_) => {
                self.named_keys.remove(name);
                Ok(())
            }
        }
    }

    pub fn get_caller(&self) -> AccountHash {
        self.account.account_hash()
    }

    pub fn get_blocktime(&self) -> BlockTime {
        self.blocktime
    }

    pub fn get_deploy_hash(&self) -> DeployHash {
        self.deploy_hash
    }

    pub fn access_rights_extend(&mut self, access_rights: HashMap<Address, HashSet<AccessRights>>) {
        self.access_rights.extend(access_rights);
    }

    pub fn access_rights(&self) -> &HashMap<Address, HashSet<AccessRights>> {
        &self.access_rights
    }

    pub fn account(&self) -> &'a Account {
        self.account
    }

    pub fn args(&self) -> &RuntimeArgs {
        &self.args
    }

    pub fn uref_address_generator(&self) -> Rc<RefCell<AddressGenerator>> {
        Rc::clone(&self.uref_address_generator)
    }

    pub fn hash_address_generator(&self) -> Rc<RefCell<AddressGenerator>> {
        Rc::clone(&self.hash_address_generator)
    }

    pub fn transfer_address_generator(&self) -> Rc<RefCell<AddressGenerator>> {
        Rc::clone(&self.transfer_address_generator)
    }

    pub(super) fn state(&self) -> Rc<RefCell<TrackingCopy<R>>> {
        Rc::clone(&self.tracking_copy)
    }

    pub fn gas_counter(&self) -> &GasCounter {
        &self.gas_counter
    }

    pub fn set_gas_counter(&mut self, new_gas_counter: GasCounter) {
        self.gas_counter = new_gas_counter;
    }

    pub fn base_key(&self) -> Key {
        self.base_key
    }

    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    pub fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Generates new deterministic hash for uses as an address.
    pub fn new_hash_address(&mut self) -> Result<[u8; KEY_HASH_LENGTH], Error> {
        Ok(self.hash_address_generator.borrow_mut().new_hash_address())
    }

    pub fn new_uref(&mut self, value: StoredValue) -> Result<URef, Error> {
        let uref = self
            .uref_address_generator
            .borrow_mut()
            .new_uref(AccessRights::READ_ADD_WRITE);
        self.insert_uref(uref);
        self.metered_write_gs(Key::URef(uref), value)?;
        Ok(uref)
    }

    /// Creates a new URef where the value it stores is CLType::Unit.
    pub(crate) fn new_unit_uref(&mut self) -> Result<URef, Error> {
        self.new_uref(StoredValue::CLValue(CLValue::unit()))
    }

    pub fn new_transfer_addr(&mut self) -> Result<TransferAddr, Error> {
        let transfer_addr = self
            .transfer_address_generator
            .borrow_mut()
            .create_address();
        Ok(TransferAddr::new(transfer_addr))
    }

    /// Puts `key` to the map of named keys of current context.
    pub fn put_key(&mut self, name: String, key: Key) -> Result<(), Error> {
        // No need to perform actual validation on the base key because an account or contract (i.e.
        // the element stored under `base_key`) is allowed to add new named keys to itself.
        let named_key_value = StoredValue::CLValue(CLValue::from_t((name.clone(), key))?);
        self.validate_value(&named_key_value)?;
        self.metered_add_gs_unsafe(self.base_key(), named_key_value)?;
        self.insert_key(name, key);
        Ok(())
    }

    pub fn read_purse_uref(&mut self, purse_uref: &URef) -> Result<Option<CLValue>, Error> {
        match self
            .tracking_copy
            .borrow_mut()
            .read(self.correlation_id, &Key::Hash(purse_uref.addr()))
            .map_err(Into::into)?
        {
            Some(stored_value) => Ok(Some(stored_value.try_into().map_err(Error::TypeMismatch)?)),
            None => Ok(None),
        }
    }

    pub fn write_purse_uref(&mut self, purse_uref: URef, cl_value: CLValue) -> Result<(), Error> {
        self.metered_write_gs_unsafe(Key::Hash(purse_uref.addr()), cl_value)
    }

    pub fn read_gs(&mut self, key: &Key) -> Result<Option<StoredValue>, Error> {
        self.validate_readable(key)?;
        self.validate_key(key)?;

        self.tracking_copy
            .borrow_mut()
            .read(self.correlation_id, key)
            .map_err(Into::into)
    }

    /// DO NOT EXPOSE THIS VIA THE FFI
    pub fn read_gs_direct(&mut self, key: &Key) -> Result<Option<StoredValue>, Error> {
        self.tracking_copy
            .borrow_mut()
            .read(self.correlation_id, key)
            .map_err(Into::into)
    }

    /// This method is a wrapper over `read_gs` in the sense that it extracts the type held by a
    /// `StoredValue` stored in the global state in a type safe manner.
    ///
    /// This is useful if you want to get the exact type from global state.
    pub fn read_gs_typed<T>(&mut self, key: &Key) -> Result<T, Error>
    where
        T: TryFrom<StoredValue>,
        T::Error: Debug,
    {
        let value = match self.read_gs(key)? {
            None => return Err(Error::KeyNotFound(*key)),
            Some(value) => value,
        };

        value.try_into().map_err(|error| {
            Error::FunctionNotFound(format!(
                "Type mismatch for value under {:?}: {:?}",
                key, error
            ))
        })
    }

    pub fn get_keys(&mut self, key_tag: &KeyTag) -> Result<BTreeSet<Key>, Error> {
        self.tracking_copy
            .borrow_mut()
            .get_keys(self.correlation_id, key_tag)
            .map_err(Into::into)
    }

    pub fn read_account(&mut self, key: &Key) -> Result<Option<StoredValue>, Error> {
        if let Key::Account(_) = key {
            self.validate_key(key)?;
            self.tracking_copy
                .borrow_mut()
                .read(self.correlation_id, key)
                .map_err(Into::into)
        } else {
            panic!("Do not use this function for reading from non-account keys")
        }
    }

    pub fn write_account(&mut self, key: Key, account: Account) -> Result<(), Error> {
        if let Key::Account(_) = key {
            self.validate_key(&key)?;
            let account_value = self.account_to_validated_value(account)?;
            self.metered_write_gs_unsafe(key, account_value)?;
            Ok(())
        } else {
            panic!("Do not use this function for writing non-account keys")
        }
    }

    pub fn write_transfer(&mut self, key: Key, value: Transfer) {
        if let Key::Transfer(_) = key {
            self.tracking_copy
                .borrow_mut()
                .write(key, StoredValue::Transfer(value));
        } else {
            panic!("Do not use this function for writing non-transfer keys")
        }
    }

    pub fn write_era_info(&mut self, key: Key, value: EraInfo) {
        if let Key::EraInfo(_) = key {
            self.tracking_copy
                .borrow_mut()
                .write(key, StoredValue::EraInfo(value));
        } else {
            panic!("Do not use this function for writing non-era-info keys")
        }
    }

    pub fn store_function(
        &mut self,
        contract: StoredValue,
    ) -> Result<[u8; KEY_HASH_LENGTH], Error> {
        self.validate_value(&contract)?;
        self.new_uref(contract).map(|uref| uref.addr())
    }

    pub fn store_function_at_hash(
        &mut self,
        contract: StoredValue,
    ) -> Result<[u8; KEY_HASH_LENGTH], Error> {
        let new_hash = self.new_hash_address()?;
        self.validate_value(&contract)?;
        self.metered_write_gs_unsafe(Key::Hash(new_hash), contract)?;
        Ok(new_hash)
    }

    pub fn insert_key(&mut self, name: String, key: Key) {
        if let Key::URef(uref) = key {
            self.insert_uref(uref);
        }
        self.named_keys.insert(name, key);
    }

    pub fn insert_uref(&mut self, uref: URef) {
        let rights = uref.access_rights();
        let entry = self
            .access_rights
            .entry(uref.addr())
            .or_insert_with(|| std::iter::empty().collect());
        entry.insert(rights);
    }

    pub fn effect(&self) -> ExecutionEffect {
        self.tracking_copy.borrow_mut().effect()
    }

    pub fn transfers(&self) -> &Vec<TransferAddr> {
        &self.transfers
    }

    pub fn transfers_mut(&mut self) -> &mut Vec<TransferAddr> {
        &mut self.transfers
    }

    fn validate_cl_value(&self, cl_value: &CLValue) -> Result<(), Error> {
        match cl_value.cl_type() {
            CLType::Bool
            | CLType::I32
            | CLType::I64
            | CLType::U8
            | CLType::U32
            | CLType::U64
            | CLType::U128
            | CLType::U256
            | CLType::U512
            | CLType::Unit
            | CLType::String
            | CLType::Option(_)
            | CLType::List(_)
            | CLType::ByteArray(..)
            | CLType::Result { .. }
            | CLType::Map { .. }
            | CLType::Tuple1(_)
            | CLType::Tuple3(_)
            | CLType::Any
            | CLType::PublicKey => Ok(()),
            CLType::Key => {
                let key: Key = cl_value.to_owned().into_t()?; // TODO: optimize?
                self.validate_key(&key)
            }
            CLType::URef => {
                let uref: URef = cl_value.to_owned().into_t()?; // TODO: optimize?
                self.validate_uref(&uref)
            }
            tuple @ CLType::Tuple2(_) if *tuple == casper_types::named_key_type() => {
                let (_name, key): (String, Key) = cl_value.to_owned().into_t()?; // TODO: optimize?
                self.validate_key(&key)
            }
            CLType::Tuple2(_) => Ok(()),
        }
    }

    /// Validates whether keys used in the `value` are not forged.
    fn validate_value(&self, value: &StoredValue) -> Result<(), Error> {
        match value {
            StoredValue::CLValue(cl_value) => self.validate_cl_value(cl_value),
            StoredValue::Account(account) => {
                // This should never happen as accounts can't be created by contracts.
                // I am putting this here for the sake of completeness.
                account
                    .named_keys()
                    .values()
                    .try_for_each(|key| self.validate_key(key))
            }
            StoredValue::ContractWasm(_) => Ok(()),
            StoredValue::Contract(contract_header) => contract_header
                .named_keys()
                .values()
                .try_for_each(|key| self.validate_key(key)),
            // TODO: anything to validate here?
            StoredValue::ContractPackage(_) => Ok(()),
            StoredValue::Transfer(_) => Ok(()),
            StoredValue::DeployInfo(_) => Ok(()),
            StoredValue::EraInfo(_) => Ok(()),
            StoredValue::Bid(_) => Ok(()),
            StoredValue::Withdraw(_) => Ok(()),
        }
    }

    /// Validates whether key is not forged (whether it can be found in the
    /// `named_keys`) and whether the version of a key that contract wants
    /// to use, has access rights that are less powerful than access rights'
    /// of the key in the `named_keys`.
    pub fn validate_key(&self, key: &Key) -> Result<(), Error> {
        let uref = match key {
            Key::URef(uref) => uref,
            _ => return Ok(()),
        };
        self.validate_uref(uref)
    }

    pub fn validate_uref(&self, uref: &URef) -> Result<(), Error> {
        if self.account.main_purse().addr() == uref.addr() {
            // If passed uref matches account's purse then we have to also validate their
            // access rights.
            let rights = self.account.main_purse().access_rights();
            let uref_rights = uref.access_rights();
            // Access rights of the passed uref, and the account's purse should match
            if rights & uref_rights == uref_rights {
                return Ok(());
            }
        }

        // Check if the `key` is known
        if uref_has_access_rights(uref, &self.access_rights) {
            Ok(())
        } else {
            Err(Error::ForgedReference(*uref))
        }
    }

    pub fn deserialize_keys(&self, bytes: Vec<u8>) -> Result<Vec<Key>, Error> {
        let keys: Vec<Key> = bytesrepr::deserialize(bytes)?;
        keys.iter().try_for_each(|k| self.validate_key(k))?;
        Ok(keys)
    }

    pub fn deserialize_urefs(&self, bytes: Vec<u8>) -> Result<Vec<URef>, Error> {
        let keys: Vec<URef> = bytesrepr::deserialize(bytes)?;
        keys.iter().try_for_each(|k| self.validate_uref(k))?;
        Ok(keys)
    }

    fn validate_readable(&self, key: &Key) -> Result<(), Error> {
        if self.is_readable(key) {
            Ok(())
        } else {
            Err(Error::InvalidAccess {
                required: AccessRights::READ,
            })
        }
    }

    fn validate_addable(&self, key: &Key) -> Result<(), Error> {
        if self.is_addable(key) {
            Ok(())
        } else {
            Err(Error::InvalidAccess {
                required: AccessRights::ADD,
            })
        }
    }

    fn validate_writeable(&self, key: &Key) -> Result<(), Error> {
        if self.is_writeable(key) {
            Ok(())
        } else {
            Err(Error::InvalidAccess {
                required: AccessRights::WRITE,
            })
        }
    }

    /// Tests whether reading from the `key` is valid.
    pub fn is_readable(&self, key: &Key) -> bool {
        match key {
            Key::Account(_) => &self.base_key() == key,
            Key::Hash(_) => true,
            Key::URef(uref) => uref.is_readable(),
            Key::Transfer(_) => true,
            Key::DeployInfo(_) => true,
            Key::EraInfo(_) => true,
            Key::Balance(_) => false,
            Key::Bid(_) => true,
            Key::Withdraw(_) => true,
            Key::Dictionary(_) => {
                // Dictionary is a special case that will not be readable by default, but the access
                // bits are verified from within API call.
                false
            }
        }
    }

    /// Tests whether addition to `key` is valid.
    pub fn is_addable(&self, key: &Key) -> bool {
        match key {
            Key::Account(_) | Key::Hash(_) => &self.base_key() == key, // ???
            Key::URef(uref) => uref.is_addable(),
            Key::Transfer(_) => false,
            Key::DeployInfo(_) => false,
            Key::EraInfo(_) => false,
            Key::Balance(_) => false,
            Key::Bid(_) => false,
            Key::Withdraw(_) => false,
            Key::Dictionary(_) => {
                // Dictionary is a special case that will not be readable by default, but the access
                // bits are verified from within API call.
                false
            }
        }
    }

    /// Tests whether writing to `key` is valid.
    pub fn is_writeable(&self, key: &Key) -> bool {
        match key {
            Key::Account(_) | Key::Hash(_) => false,
            Key::URef(uref) => uref.is_writeable(),
            Key::Transfer(_) => false,
            Key::DeployInfo(_) => false,
            Key::EraInfo(_) => false,
            Key::Balance(_) => false,
            Key::Bid(_) => false,
            Key::Withdraw(_) => false,
            Key::Dictionary(_) => {
                // Dictionary is a special case that will not be readable by default, but the access
                // bits are verified from within API call.
                false
            }
        }
    }

    /// Safely charge the specified amount of gas, up to the available gas limit.
    ///
    /// Returns [`Error::GasLimit`] if gas limit exceeded and `()` if not.
    /// Intuition about the return value sense is to answer the question 'are we
    /// allowed to continue?'
    pub fn charge_gas(&mut self, amount: u64) -> Result<(), Error> {
        self.gas_counter.add(amount)?;
        Ok(())
    }

    pub fn charge_gas_large(&mut self, amount: Gas) -> Result<(), Error> {
        self.gas_counter.add_large(amount)?;
        Ok(())
    }

    /// Checks if we are calling a system contract.
    pub(crate) fn is_system_contract(&self) -> bool {
        if let Some(hash) = self.base_key().into_hash() {
            let system_contracts = self.protocol_data().system_contracts();
            if system_contracts.contains(&hash.into()) {
                return true;
            }
        }
        false
    }

    /// Charges gas for specified amount of bytes used.
    fn charge_gas_storage(&mut self, bytes_count: usize) -> Result<(), Error> {
        if self.is_system_contract() {
            // Don't charge storage used while executing a system contract.
            return Ok(());
        }

        let storage_costs = self.protocol_data().wasm_config().storage_costs();

        let gas_cost = storage_costs.calculate_gas_cost(bytes_count);

        self.charge_gas_large(gas_cost)
    }

    /// Charges gas for using a host system contract's entrypoint.
    pub(crate) fn charge_system_contract_call<T>(&mut self, call_cost: T) -> Result<(), Error>
    where
        T: Into<u64>,
    {
        if self.account.account_hash() == PublicKey::System.to_account_hash() {
            // Don't try to charge a system account for calling a system contract's entry point.
            // This will make sure that (for example) calling a mint's transfer from within auction
            // wouldn't try to incur cost to system account.
            return Ok(());
        }
        self.charge_gas(call_cost.into())
    }

    /// Writes data to global state with a measurement
    pub(crate) fn metered_write_gs_unsafe<K, V>(&mut self, key: K, value: V) -> Result<(), Error>
    where
        K: Into<Key>,
        V: Into<StoredValue>,
    {
        let stored_value = value.into();

        // Charge for amount as measured by serialized length
        let bytes_count = stored_value.serialized_length();
        self.charge_gas_storage(bytes_count)?;

        self.tracking_copy
            .borrow_mut()
            .write(key.into(), stored_value);
        Ok(())
    }

    pub fn metered_write_gs<T>(&mut self, key: Key, value: T) -> Result<(), Error>
    where
        T: Into<StoredValue>,
    {
        let stored_value = value.into();
        self.validate_writeable(&key)?;
        self.validate_key(&key)?;
        self.validate_value(&stored_value)?;
        self.metered_write_gs_unsafe(key, stored_value)?;
        Ok(())
    }

    pub(crate) fn metered_add_gs_unsafe(
        &mut self,
        key: Key,
        value: StoredValue,
    ) -> Result<(), Error> {
        let value_bytes_count = value.serialized_length();
        self.charge_gas_storage(value_bytes_count)?;

        match self
            .tracking_copy
            .borrow_mut()
            .add(self.correlation_id, key, value)
        {
            Err(storage_error) => Err(storage_error.into()),
            Ok(AddResult::Success) => Ok(()),
            Ok(AddResult::KeyNotFound(key)) => Err(Error::KeyNotFound(key)),
            Ok(AddResult::TypeMismatch(type_mismatch)) => Err(Error::TypeMismatch(type_mismatch)),
            Ok(AddResult::Serialization(error)) => Err(Error::BytesRepr(error)),
        }
    }

    /// Adds `value` to the `key`. The premise for being able to `add` value is
    /// that the type of it value can be added (is a Monoid). If the
    /// values can't be added, either because they're not a Monoid or if the
    /// value stored under `key` has different type, then `TypeMismatch`
    /// errors is returned.
    pub(crate) fn metered_add_gs<K, V>(&mut self, key: K, value: V) -> Result<(), Error>
    where
        K: Into<Key>,
        V: Into<StoredValue>,
    {
        let key = key.into();
        let value = value.into();
        self.validate_addable(&key)?;
        self.validate_key(&key)?;
        self.validate_value(&value)?;
        self.metered_add_gs_unsafe(key, value)
    }

    pub fn add_associated_key(
        &mut self,
        account_hash: AccountHash,
        weight: Weight,
    ) -> Result<(), Error> {
        // Check permission to modify associated keys
        if !self.is_valid_context() {
            // Exit early with error to avoid mutations
            return Err(AddKeyFailure::PermissionDenied.into());
        }

        if !self
            .account()
            .can_manage_keys_with(&self.authorization_keys)
        {
            // Exit early if authorization keys weight doesn't exceed required
            // key management threshold
            return Err(AddKeyFailure::PermissionDenied.into());
        }

        // Converts an account's public key into a URef
        let key = Key::Account(self.account().account_hash());

        // Take an account out of the global state
        let account = {
            let mut account: Account = self.read_gs_typed(&key)?;
            // Exit early in case of error without updating global state
            account
                .add_associated_key(account_hash, weight)
                .map_err(Error::from)?;
            account
        };

        let account_value = self.account_to_validated_value(account)?;

        self.metered_write_gs_unsafe(key, account_value)?;

        Ok(())
    }

    pub fn remove_associated_key(&mut self, account_hash: AccountHash) -> Result<(), Error> {
        // Check permission to modify associated keys
        if !self.is_valid_context() {
            // Exit early with error to avoid mutations
            return Err(RemoveKeyFailure::PermissionDenied.into());
        }

        if !self
            .account()
            .can_manage_keys_with(&self.authorization_keys)
        {
            // Exit early if authorization keys weight doesn't exceed required
            // key management threshold
            return Err(RemoveKeyFailure::PermissionDenied.into());
        }

        // Converts an account's public key into a URef
        let key = Key::Account(self.account().account_hash());

        // Take an account out of the global state
        let mut account: Account = self.read_gs_typed(&key)?;

        // Exit early in case of error without updating global state
        account
            .remove_associated_key(account_hash)
            .map_err(Error::from)?;

        let account_value = self.account_to_validated_value(account)?;

        self.metered_write_gs_unsafe(key, account_value)?;

        Ok(())
    }

    pub fn update_associated_key(
        &mut self,
        account_hash: AccountHash,
        weight: Weight,
    ) -> Result<(), Error> {
        // Check permission to modify associated keys
        if !self.is_valid_context() {
            // Exit early with error to avoid mutations
            return Err(UpdateKeyFailure::PermissionDenied.into());
        }

        if !self
            .account()
            .can_manage_keys_with(&self.authorization_keys)
        {
            // Exit early if authorization keys weight doesn't exceed required
            // key management threshold
            return Err(UpdateKeyFailure::PermissionDenied.into());
        }

        // Converts an account's public key into a URef
        let key = Key::Account(self.account().account_hash());

        // Take an account out of the global state
        let mut account: Account = self.read_gs_typed(&key)?;

        // Exit early in case of error without updating global state
        account
            .update_associated_key(account_hash, weight)
            .map_err(Error::from)?;

        let account_value = self.account_to_validated_value(account)?;

        self.metered_write_gs_unsafe(key, account_value)?;

        Ok(())
    }

    pub fn set_action_threshold(
        &mut self,
        action_type: ActionType,
        threshold: Weight,
    ) -> Result<(), Error> {
        // Check permission to modify associated keys
        if !self.is_valid_context() {
            // Exit early with error to avoid mutations
            return Err(SetThresholdFailure::PermissionDeniedError.into());
        }

        if !self
            .account()
            .can_manage_keys_with(&self.authorization_keys)
        {
            // Exit early if authorization keys weight doesn't exceed required
            // key management threshold
            return Err(SetThresholdFailure::PermissionDeniedError.into());
        }

        // Converts an account's public key into a URef
        let key = Key::Account(self.account().account_hash());

        // Take an account out of the global state
        let mut account: Account = self.read_gs_typed(&key)?;

        // Exit early in case of error without updating global state
        account
            .set_action_threshold(action_type, threshold)
            .map_err(Error::from)?;

        let account_value = self.account_to_validated_value(account)?;

        self.metered_write_gs_unsafe(key, account_value)?;

        Ok(())
    }

    pub fn protocol_data(&self) -> &ProtocolData {
        &self.protocol_data
    }

    /// Creates validated instance of `StoredValue` from `account`.
    fn account_to_validated_value(&self, account: Account) -> Result<StoredValue, Error> {
        let value = StoredValue::Account(account);
        self.validate_value(&value)?;
        Ok(value)
    }

    /// Checks if the account context is valid.
    fn is_valid_context(&self) -> bool {
        self.base_key() == Key::Account(self.account().account_hash())
    }

    /// Gets main purse id
    pub fn get_main_purse(&self) -> Result<URef, Error> {
        if !self.is_valid_context() {
            return Err(Error::InvalidContext);
        }
        Ok(self.account().main_purse())
    }

    /// Gets entry point type.
    pub fn entry_point_type(&self) -> EntryPointType {
        self.entry_point_type
    }

    /// Gets given contract package with its access_key validated against current context.
    pub(crate) fn get_validated_contract_package(
        &mut self,
        package_hash: ContractPackageHash,
    ) -> Result<ContractPackage, Error> {
        let package_hash_key = Key::from(package_hash);
        self.validate_key(&package_hash_key)?;
        let contract_package: ContractPackage = self.read_gs_typed(&Key::from(package_hash))?;
        self.validate_uref(&contract_package.access_key())?;
        Ok(contract_package)
    }

    pub(crate) fn dictionary_get(
        &mut self,
        uref: URef,
        key_bytes: &[u8],
    ) -> Result<Option<CLValue>, Error> {
        self.validate_readable(&uref.into())?;
        self.validate_key(&uref.into())?;

        let dictionary_key = Key::dictionary(uref, key_bytes);

        let maybe_stored_value = self
            .tracking_copy
            .borrow_mut()
            .read(self.correlation_id, &dictionary_key)
            .map_err(Into::into)?;

        if let Some(stored_value) = maybe_stored_value {
            let cl_value_indirect: CLValue =
                stored_value.try_into().map_err(Error::TypeMismatch)?;
            let dictionary_value: DictionaryValue =
                cl_value_indirect.into_t().map_err(Error::from)?;
            let cl_value = dictionary_value.into_cl_value();
            Ok(Some(cl_value))
        } else {
            Ok(None)
        }
    }

    pub fn dictionary_put(
        &mut self,
        uref: URef,
        key_bytes: &[u8],
        cl_value: CLValue,
    ) -> Result<(), Error> {
        self.validate_writeable(&uref.into())?;
        self.validate_uref(&uref)?;

        self.validate_cl_value(&cl_value)?;

        let wrapped_cl_value = {
            let dictionary_value =
                DictionaryValue::new(cl_value, key_bytes.to_vec(), uref.addr().to_vec());
            CLValue::from_t(dictionary_value).map_err(Error::from)?
        };

        let dictionary_key = Key::dictionary(uref, key_bytes);
        self.metered_write_gs_unsafe(dictionary_key, wrapped_cl_value)?;
        Ok(())
    }
}
