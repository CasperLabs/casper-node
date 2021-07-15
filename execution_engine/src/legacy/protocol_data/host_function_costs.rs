use casper_types::bytesrepr::{self, FromBytes};

use crate::shared::host_function_costs::{HostFunction, HostFunctionCosts};

pub(crate) type LegacyCost = u32;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct LegacyHostFunction<T> {
    /// How much user is charged for cost only
    cost: LegacyCost,
    arguments: T,
}

impl<T> FromBytes for LegacyHostFunction<T>
where
    T: Default + AsMut<[LegacyCost]>,
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (cost, mut bytes) = FromBytes::from_bytes(bytes)?;
        let mut arguments = T::default();
        let arguments_mut = arguments.as_mut();
        for ith_argument in arguments_mut {
            let (cost, rem) = FromBytes::from_bytes(bytes)?;
            *ith_argument = cost;
            bytes = rem;
        }
        Ok((Self { cost, arguments }, bytes))
    }
}

impl<T> From<LegacyHostFunction<T>> for HostFunction<T> {
    fn from(value: LegacyHostFunction<T>) -> Self {
        HostFunction::new(value.cost, value.arguments)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct LegacyHostFunctionCosts {
    pub read_value: LegacyHostFunction<[LegacyCost; 3]>,
    pub read_value_local: LegacyHostFunction<[LegacyCost; 3]>,
    pub write: LegacyHostFunction<[LegacyCost; 4]>,
    pub write_local: LegacyHostFunction<[LegacyCost; 4]>,
    pub add: LegacyHostFunction<[LegacyCost; 4]>,
    pub new_uref: LegacyHostFunction<[LegacyCost; 3]>,
    pub load_named_keys: LegacyHostFunction<[LegacyCost; 2]>,
    pub ret: LegacyHostFunction<[LegacyCost; 2]>,
    pub get_key: LegacyHostFunction<[LegacyCost; 5]>,
    pub has_key: LegacyHostFunction<[LegacyCost; 2]>,
    pub put_key: LegacyHostFunction<[LegacyCost; 4]>,
    pub remove_key: LegacyHostFunction<[LegacyCost; 2]>,
    pub revert: LegacyHostFunction<[LegacyCost; 1]>,
    pub is_valid_uref: LegacyHostFunction<[LegacyCost; 2]>,
    pub add_associated_key: LegacyHostFunction<[LegacyCost; 3]>,
    pub remove_associated_key: LegacyHostFunction<[LegacyCost; 2]>,
    pub update_associated_key: LegacyHostFunction<[LegacyCost; 3]>,
    pub set_action_threshold: LegacyHostFunction<[LegacyCost; 2]>,
    pub get_caller: LegacyHostFunction<[LegacyCost; 1]>,
    pub get_blocktime: LegacyHostFunction<[LegacyCost; 1]>,
    pub create_purse: LegacyHostFunction<[LegacyCost; 2]>,
    pub transfer_to_account: LegacyHostFunction<[LegacyCost; 7]>,
    pub transfer_from_purse_to_account: LegacyHostFunction<[LegacyCost; 9]>,
    pub transfer_from_purse_to_purse: LegacyHostFunction<[LegacyCost; 8]>,
    pub get_balance: LegacyHostFunction<[LegacyCost; 3]>,
    pub get_phase: LegacyHostFunction<[LegacyCost; 1]>,
    pub get_system_contract: LegacyHostFunction<[LegacyCost; 3]>,
    pub get_main_purse: LegacyHostFunction<[LegacyCost; 1]>,
    pub read_host_buffer: LegacyHostFunction<[LegacyCost; 3]>,
    pub create_contract_package_at_hash: LegacyHostFunction<[LegacyCost; 2]>,
    pub create_contract_user_group: LegacyHostFunction<[LegacyCost; 8]>,
    pub add_contract_version: LegacyHostFunction<[LegacyCost; 10]>,
    pub disable_contract_version: LegacyHostFunction<[LegacyCost; 4]>,
    pub call_contract: LegacyHostFunction<[LegacyCost; 7]>,
    pub call_versioned_contract: LegacyHostFunction<[LegacyCost; 9]>,
    pub get_named_arg_size: LegacyHostFunction<[LegacyCost; 3]>,
    pub get_named_arg: LegacyHostFunction<[LegacyCost; 4]>,
    pub remove_contract_user_group: LegacyHostFunction<[LegacyCost; 4]>,
    pub provision_contract_user_group_uref: LegacyHostFunction<[LegacyCost; 5]>,
    pub remove_contract_user_group_urefs: LegacyHostFunction<[LegacyCost; 6]>,
    pub print: LegacyHostFunction<[LegacyCost; 2]>,
    pub blake2b: LegacyHostFunction<[LegacyCost; 4]>,
}

impl From<LegacyHostFunctionCosts> for HostFunctionCosts {
    fn from(value: LegacyHostFunctionCosts) -> Self {
        HostFunctionCosts {
            read_value: HostFunction::from(value.read_value),
            dictionary_get: HostFunction::from(value.read_value_local),
            write: HostFunction::from(value.write),
            dictionary_put: HostFunction::from(value.write_local),
            add: HostFunction::from(value.add),
            new_uref: HostFunction::from(value.new_uref),
            load_named_keys: HostFunction::from(value.load_named_keys),
            ret: HostFunction::from(value.ret),
            get_key: HostFunction::from(value.get_key),
            has_key: HostFunction::from(value.has_key),
            put_key: HostFunction::from(value.put_key),
            remove_key: HostFunction::from(value.remove_key),
            revert: HostFunction::from(value.revert),
            is_valid_uref: HostFunction::from(value.is_valid_uref),
            add_associated_key: HostFunction::from(value.add_associated_key),
            remove_associated_key: HostFunction::from(value.remove_associated_key),
            update_associated_key: HostFunction::from(value.update_associated_key),
            set_action_threshold: HostFunction::from(value.set_action_threshold),
            get_caller: HostFunction::from(value.get_caller),
            get_blocktime: HostFunction::from(value.get_blocktime),
            create_purse: HostFunction::from(value.create_purse),
            transfer_to_account: HostFunction::from(value.transfer_to_account),
            transfer_from_purse_to_account: HostFunction::from(
                value.transfer_from_purse_to_account,
            ),
            transfer_from_purse_to_purse: HostFunction::from(value.transfer_from_purse_to_purse),
            get_balance: HostFunction::from(value.get_balance),
            get_phase: HostFunction::from(value.get_phase),
            get_system_contract: HostFunction::from(value.get_system_contract),
            get_main_purse: HostFunction::from(value.get_main_purse),
            read_host_buffer: HostFunction::from(value.read_host_buffer),
            create_contract_package_at_hash: HostFunction::from(
                value.create_contract_package_at_hash,
            ),
            create_contract_user_group: HostFunction::from(value.create_contract_user_group),
            add_contract_version: HostFunction::from(value.add_contract_version),
            disable_contract_version: HostFunction::from(value.disable_contract_version),
            call_contract: HostFunction::from(value.call_contract),
            call_versioned_contract: HostFunction::from(value.call_versioned_contract),
            get_named_arg_size: HostFunction::from(value.get_named_arg_size),
            get_named_arg: HostFunction::from(value.get_named_arg),
            remove_contract_user_group: HostFunction::from(value.remove_contract_user_group),
            provision_contract_user_group_uref: HostFunction::from(
                value.provision_contract_user_group_uref,
            ),
            remove_contract_user_group_urefs: HostFunction::from(
                value.remove_contract_user_group_urefs,
            ),
            print: HostFunction::from(value.print),
            blake2b: HostFunction::from(value.blake2b),
        }
    }
}

impl FromBytes for LegacyHostFunctionCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (read_value, bytes) = FromBytes::from_bytes(bytes)?;
        let (read_value_local, bytes) = FromBytes::from_bytes(bytes)?;
        let (write, bytes) = FromBytes::from_bytes(bytes)?;
        let (write_local, bytes) = FromBytes::from_bytes(bytes)?;
        let (add, bytes) = FromBytes::from_bytes(bytes)?;
        let (new_uref, bytes) = FromBytes::from_bytes(bytes)?;
        let (load_named_keys, bytes) = FromBytes::from_bytes(bytes)?;
        let (ret, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (has_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (put_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (remove_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (revert, bytes) = FromBytes::from_bytes(bytes)?;
        let (is_valid_uref, bytes) = FromBytes::from_bytes(bytes)?;
        let (add_associated_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (remove_associated_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (update_associated_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (set_action_threshold, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_caller, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_blocktime, bytes) = FromBytes::from_bytes(bytes)?;
        let (create_purse, bytes) = FromBytes::from_bytes(bytes)?;
        let (transfer_to_account, bytes) = FromBytes::from_bytes(bytes)?;
        let (transfer_from_purse_to_account, bytes) = FromBytes::from_bytes(bytes)?;
        let (transfer_from_purse_to_purse, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_balance, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_phase, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_system_contract, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_main_purse, bytes) = FromBytes::from_bytes(bytes)?;
        let (read_host_buffer, bytes) = FromBytes::from_bytes(bytes)?;
        let (create_contract_package_at_hash, bytes) = FromBytes::from_bytes(bytes)?;
        let (create_contract_user_group, bytes) = FromBytes::from_bytes(bytes)?;
        let (add_contract_version, bytes) = FromBytes::from_bytes(bytes)?;
        let (disable_contract_version, bytes) = FromBytes::from_bytes(bytes)?;
        let (call_contract, bytes) = FromBytes::from_bytes(bytes)?;
        let (call_versioned_contract, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_named_arg_size, bytes) = FromBytes::from_bytes(bytes)?;
        let (get_named_arg, bytes) = FromBytes::from_bytes(bytes)?;
        let (remove_contract_user_group, bytes) = FromBytes::from_bytes(bytes)?;
        let (provision_contract_user_group_uref, bytes) = FromBytes::from_bytes(bytes)?;
        let (remove_contract_user_group_urefs, bytes) = FromBytes::from_bytes(bytes)?;
        let (print, bytes) = FromBytes::from_bytes(bytes)?;
        let (blake2b, bytes) = FromBytes::from_bytes(bytes)?;

        let legacy_host_function_costs = LegacyHostFunctionCosts {
            read_value,
            read_value_local,
            write,
            write_local,
            add,
            new_uref,
            load_named_keys,
            ret,
            get_key,
            has_key,
            put_key,
            remove_key,
            revert,
            is_valid_uref,
            add_associated_key,
            remove_associated_key,
            update_associated_key,
            set_action_threshold,
            get_caller,
            get_blocktime,
            create_purse,
            transfer_to_account,
            transfer_from_purse_to_account,
            transfer_from_purse_to_purse,
            get_balance,
            get_phase,
            get_system_contract,
            get_main_purse,
            read_host_buffer,
            create_contract_package_at_hash,
            create_contract_user_group,
            add_contract_version,
            disable_contract_version,
            call_contract,
            call_versioned_contract,
            get_named_arg_size,
            get_named_arg,
            remove_contract_user_group,
            provision_contract_user_group_uref,
            remove_contract_user_group_urefs,
            print,
            blake2b,
        };

        Ok((legacy_host_function_costs, bytes))
    }
}
