use std::{collections::BTreeSet, convert::TryFrom};

use wasmi::{Externals, RuntimeArgs, RuntimeValue, Trap};

use casper_types::{
    account,
    account::AccountHash,
    api_error,
    bytesrepr::{self, ToBytes},
    contracts::{ContractPackageStatus, EntryPoints, NamedKeys},
    system::auction::EraInfo,
    ContractHash, ContractPackageHash, ContractVersion, EraId, Group, Key, URef, U512,
};

use super::{args::Args, scoped_instrumenter::ScopedInstrumenter, Error, Runtime};
use crate::{
    core::resolvers::v1_function_index::FunctionIndex,
    shared::{
        host_function_costs::{Cost, HostFunction, DEFAULT_HOST_FUNCTION_NEW_DICTIONARY},
        stored_value::StoredValue,
    },
    storage::global_state::StateReader,
};

impl<'a, R> Externals for Runtime<'a, R>
where
    R: StateReader<Key, StoredValue>,
    R::Error: Into<Error>,
{
    fn invoke_index(
        &mut self,
        index: usize,
        args: RuntimeArgs,
    ) -> Result<Option<RuntimeValue>, Trap> {
        let func = FunctionIndex::try_from(index).expect("unknown function index");
        let mut scoped_instrumenter = ScopedInstrumenter::new(func);

        let host_function_costs = self
            .protocol_data()
            .wasm_config()
            .take_host_function_costs();

        match func {
            FunctionIndex::ReadFuncIndex => {
                // args(0) = pointer to key in Wasm memory
                // args(1) = size of key in Wasm memory
                // args(2) = pointer to output size (output param)
                let (key_ptr, key_size, output_size_ptr) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.read_value,
                    [key_ptr, key_size, output_size_ptr],
                )?;
                let ret = self.read(key_ptr, key_size, output_size_ptr)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::LoadNamedKeysFuncIndex => {
                // args(0) = pointer to amount of keys (output)
                // args(1) = pointer to amount of serialized bytes (output)
                let (total_keys_ptr, result_size_ptr) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.load_named_keys,
                    [total_keys_ptr, result_size_ptr],
                )?;
                let ret = self.load_named_keys(
                    total_keys_ptr,
                    result_size_ptr,
                    &mut scoped_instrumenter,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::WriteFuncIndex => {
                // args(0) = pointer to key in Wasm memory
                // args(1) = size of key
                // args(2) = pointer to value
                // args(3) = size of value
                let (key_ptr, key_size, value_ptr, value_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.write,
                    [key_ptr, key_size, value_ptr, value_size],
                )?;
                scoped_instrumenter.add_property("value_size", value_size);
                self.write(key_ptr, key_size, value_ptr, value_size)?;
                Ok(None)
            }

            FunctionIndex::AddFuncIndex => {
                // args(0) = pointer to key in Wasm memory
                // args(1) = size of key
                // args(2) = pointer to value
                // args(3) = size of value
                let (key_ptr, key_size, value_ptr, value_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.add,
                    [key_ptr, key_size, value_ptr, value_size],
                )?;
                self.add(key_ptr, key_size, value_ptr, value_size)?;
                Ok(None)
            }

            FunctionIndex::NewFuncIndex => {
                // args(0) = pointer to uref destination in Wasm memory
                // args(1) = pointer to initial value
                // args(2) = size of initial value
                let (uref_ptr, value_ptr, value_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.new_uref,
                    [uref_ptr, value_ptr, value_size],
                )?;
                scoped_instrumenter.add_property("value_size", value_size);
                self.new_uref(uref_ptr, value_ptr, value_size)?;
                Ok(None)
            }

            FunctionIndex::RetFuncIndex => {
                // args(0) = pointer to value
                // args(1) = size of value
                let (value_ptr, value_size) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.ret, [value_ptr, value_size])?;
                scoped_instrumenter.add_property("value_size", value_size);
                Err(self.ret(value_ptr, value_size as usize, &mut scoped_instrumenter))
            }

            FunctionIndex::GetKeyFuncIndex => {
                // args(0) = pointer to key name in Wasm memory
                // args(1) = size of key name
                // args(2) = pointer to output buffer for serialized key
                // args(3) = size of output buffer
                // args(4) = pointer to bytes written
                let (name_ptr, name_size, output_ptr, output_size, bytes_written) =
                    Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.get_key,
                    [name_ptr, name_size, output_ptr, output_size, bytes_written],
                )?;
                scoped_instrumenter.add_property("name_size", name_size);
                let ret = self.load_key(
                    name_ptr,
                    name_size,
                    output_ptr,
                    output_size as usize,
                    bytes_written,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::HasKeyFuncIndex => {
                // args(0) = pointer to key name in Wasm memory
                // args(1) = size of key name
                let (name_ptr, name_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.has_key,
                    [name_ptr, name_size],
                )?;
                scoped_instrumenter.add_property("name_size", name_size);
                let result = self.has_key(name_ptr, name_size)?;
                Ok(Some(RuntimeValue::I32(result)))
            }

            FunctionIndex::PutKeyFuncIndex => {
                // args(0) = pointer to key name in Wasm memory
                // args(1) = size of key name
                // args(2) = pointer to key in Wasm memory
                // args(3) = size of key
                let (name_ptr, name_size, key_ptr, key_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.put_key,
                    [name_ptr, name_size, key_ptr, key_size],
                )?;
                scoped_instrumenter.add_property("name_size", name_size);
                self.put_key(name_ptr, name_size, key_ptr, key_size)?;
                Ok(None)
            }

            FunctionIndex::RemoveKeyFuncIndex => {
                // args(0) = pointer to key name in Wasm memory
                // args(1) = size of key name
                let (name_ptr, name_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.remove_key,
                    [name_ptr, name_size],
                )?;
                scoped_instrumenter.add_property("name_size", name_size);
                self.remove_key(name_ptr, name_size)?;
                Ok(None)
            }

            FunctionIndex::GetCallerIndex => {
                // args(0) = pointer where a size of serialized bytes will be stored
                let (output_size,) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.get_caller, [output_size])?;
                let ret = self.get_caller(output_size)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::GetBlocktimeIndex => {
                // args(0) = pointer to Wasm memory where to write.
                let (dest_ptr,) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.get_blocktime, [dest_ptr])?;
                self.get_blocktime(dest_ptr)?;
                Ok(None)
            }

            FunctionIndex::GasFuncIndex => {
                let (gas_arg,): (u32,) = Args::parse(args)?;
                // Gas is special cased internal host function and for accounting purposes it isn't
                // represented in protocol data.
                self.gas(gas_arg as u64)?;
                Ok(None)
            }

            FunctionIndex::IsValidURefFnIndex => {
                // args(0) = pointer to value to validate
                // args(1) = size of value
                let (uref_ptr, uref_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.is_valid_uref,
                    [uref_ptr, uref_size],
                )?;
                Ok(Some(RuntimeValue::I32(i32::from(
                    self.is_valid_uref(uref_ptr, uref_size)?,
                ))))
            }

            FunctionIndex::RevertFuncIndex => {
                // args(0) = status u32
                let (status,) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.revert, [status])?;
                Err(self.revert(status))
            }

            FunctionIndex::AddAssociatedKeyFuncIndex => {
                // args(0) = pointer to array of bytes of an account hash
                // args(1) = size of an account hash
                // args(2) = weight of the key
                let (account_hash_ptr, account_hash_size, weight_value) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.add_associated_key,
                    [account_hash_ptr, account_hash_size, weight_value as Cost],
                )?;
                let value = self.add_associated_key(
                    account_hash_ptr,
                    account_hash_size as usize,
                    weight_value,
                )?;
                Ok(Some(RuntimeValue::I32(value)))
            }

            FunctionIndex::RemoveAssociatedKeyFuncIndex => {
                // args(0) = pointer to array of bytes of an account hash
                // args(1) = size of an account hash
                let (account_hash_ptr, account_hash_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.remove_associated_key,
                    [account_hash_ptr, account_hash_size],
                )?;
                let value =
                    self.remove_associated_key(account_hash_ptr, account_hash_size as usize)?;
                Ok(Some(RuntimeValue::I32(value)))
            }

            FunctionIndex::UpdateAssociatedKeyFuncIndex => {
                // args(0) = pointer to array of bytes of an account hash
                // args(1) = size of an account hash
                // args(2) = weight of the key
                let (account_hash_ptr, account_hash_size, weight_value) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.update_associated_key,
                    [account_hash_ptr, account_hash_size, weight_value as Cost],
                )?;
                let value = self.update_associated_key(
                    account_hash_ptr,
                    account_hash_size as usize,
                    weight_value,
                )?;
                Ok(Some(RuntimeValue::I32(value)))
            }

            FunctionIndex::SetActionThresholdFuncIndex => {
                // args(0) = action type
                // args(1) = new threshold
                let (action_type_value, threshold_value) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.set_action_threshold,
                    [action_type_value, threshold_value as Cost],
                )?;
                let value = self.set_action_threshold(action_type_value, threshold_value)?;
                Ok(Some(RuntimeValue::I32(value)))
            }

            FunctionIndex::CreatePurseIndex => {
                // args(0) = pointer to array for return value
                // args(1) = length of array for return value
                let (dest_ptr, dest_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.create_purse,
                    [dest_ptr, dest_size],
                )?;
                let purse = self.create_purse()?;
                let purse_bytes = purse.into_bytes().map_err(Error::BytesRepr)?;
                assert_eq!(dest_size, purse_bytes.len() as u32);
                self.memory
                    .set(dest_ptr, &purse_bytes)
                    .map_err(|e| Error::Interpreter(e.into()))?;
                Ok(Some(RuntimeValue::I32(0)))
            }

            FunctionIndex::TransferToAccountIndex => {
                // args(0) = pointer to array of bytes of an account hash
                // args(1) = length of array of bytes of an account hash
                // args(2) = pointer to array of bytes of an amount
                // args(3) = length of array of bytes of an amount
                // args(4) = pointer to array of bytes of an id
                // args(5) = length of array of bytes of an id
                // args(6) = pointer to a value where new value will be set
                let (key_ptr, key_size, amount_ptr, amount_size, id_ptr, id_size, result_ptr) =
                    Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.transfer_to_account,
                    [
                        key_ptr,
                        key_size,
                        amount_ptr,
                        amount_size,
                        id_ptr,
                        id_size,
                        result_ptr,
                    ],
                )?;
                let account_hash: AccountHash = {
                    let bytes = self.bytes_from_mem(key_ptr, key_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };
                let amount: U512 = {
                    let bytes = self.bytes_from_mem(amount_ptr, amount_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };
                let id: Option<u64> = {
                    let bytes = self.bytes_from_mem(id_ptr, id_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };

                let ret = match self.transfer_to_account(account_hash, amount, id)? {
                    Ok(transferred_to) => {
                        let result_value: u32 = transferred_to as u32;
                        let result_value_bytes = result_value.to_le_bytes();
                        self.memory
                            .set(result_ptr, &result_value_bytes)
                            .map_err(|error| Error::Interpreter(error.into()))?;
                        Ok(())
                    }
                    Err(api_error) => Err(api_error),
                };
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::TransferFromPurseToAccountIndex => {
                // args(0) = pointer to array of bytes in Wasm memory of a source purse
                // args(1) = length of array of bytes in Wasm memory of a source purse
                // args(2) = pointer to array of bytes in Wasm memory of an account hash
                // args(3) = length of array of bytes in Wasm memory of an account hash
                // args(4) = pointer to array of bytes in Wasm memory of an amount
                // args(5) = length of array of bytes in Wasm memory of an amount
                // args(6) = pointer to array of bytes in Wasm memory of an id
                // args(7) = length of array of bytes in Wasm memory of an id
                // args(8) = pointer to a value where value of `TransferredTo` enum will be set
                let (
                    source_ptr,
                    source_size,
                    key_ptr,
                    key_size,
                    amount_ptr,
                    amount_size,
                    id_ptr,
                    id_size,
                    result_ptr,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.transfer_from_purse_to_account,
                    [
                        source_ptr,
                        source_size,
                        key_ptr,
                        key_size,
                        amount_ptr,
                        amount_size,
                        id_ptr,
                        id_size,
                        result_ptr,
                    ],
                )?;
                let source_purse = {
                    let bytes = self.bytes_from_mem(source_ptr, source_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };
                let account_hash: AccountHash = {
                    let bytes = self.bytes_from_mem(key_ptr, key_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };
                let amount: U512 = {
                    let bytes = self.bytes_from_mem(amount_ptr, amount_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };
                let id: Option<u64> = {
                    let bytes = self.bytes_from_mem(id_ptr, id_size as usize)?;
                    bytesrepr::deserialize(bytes).map_err(Error::BytesRepr)?
                };
                let ret = match self.transfer_from_purse_to_account(
                    source_purse,
                    account_hash,
                    amount,
                    id,
                )? {
                    Ok(transferred_to) => {
                        let result_value: u32 = transferred_to as u32;
                        let result_value_bytes = result_value.to_le_bytes();
                        self.memory
                            .set(result_ptr, &result_value_bytes)
                            .map_err(|error| Error::Interpreter(error.into()))?;
                        Ok(())
                    }
                    Err(api_error) => Err(api_error),
                };
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::TransferFromPurseToPurseIndex => {
                // args(0) = pointer to array of bytes in Wasm memory of a source purse
                // args(1) = length of array of bytes in Wasm memory of a source purse
                // args(2) = pointer to array of bytes in Wasm memory of a target purse
                // args(3) = length of array of bytes in Wasm memory of a target purse
                // args(4) = pointer to array of bytes in Wasm memory of an amount
                // args(5) = length of array of bytes in Wasm memory of an amount
                // args(6) = pointer to array of bytes in Wasm memory of an id
                // args(7) = length of array of bytes in Wasm memory of an id
                let (
                    source_ptr,
                    source_size,
                    target_ptr,
                    target_size,
                    amount_ptr,
                    amount_size,
                    id_ptr,
                    id_size,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.transfer_from_purse_to_purse,
                    [
                        source_ptr,
                        source_size,
                        target_ptr,
                        target_size,
                        amount_ptr,
                        amount_size,
                        id_ptr,
                        id_size,
                    ],
                )?;
                let ret = self.transfer_from_purse_to_purse(
                    source_ptr,
                    source_size,
                    target_ptr,
                    target_size,
                    amount_ptr,
                    amount_size,
                    id_ptr,
                    id_size,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::GetBalanceIndex => {
                // args(0) = pointer to purse input
                // args(1) = length of purse
                // args(2) = pointer to output size (output)
                let (ptr, ptr_size, output_size_ptr) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.get_balance,
                    [ptr, ptr_size, output_size_ptr],
                )?;
                let ret = self.get_balance_host_buffer(ptr, ptr_size as usize, output_size_ptr)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::GetPhaseIndex => {
                // args(0) = pointer to Wasm memory where to write.
                let (dest_ptr,) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.get_phase, [dest_ptr])?;
                self.get_phase(dest_ptr)?;
                Ok(None)
            }

            FunctionIndex::GetSystemContractIndex => {
                // args(0) = system contract index
                // args(1) = dest pointer for storing serialized result
                // args(2) = dest pointer size
                let (system_contract_index, dest_ptr, dest_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.get_system_contract,
                    [system_contract_index, dest_ptr, dest_size],
                )?;
                let ret = self.get_system_contract(system_contract_index, dest_ptr, dest_size)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::GetMainPurseIndex => {
                // args(0) = pointer to Wasm memory where to write.
                let (dest_ptr,) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.get_main_purse, [dest_ptr])?;
                self.get_main_purse(dest_ptr)?;
                Ok(None)
            }

            FunctionIndex::ReadHostBufferIndex => {
                // args(0) = pointer to Wasm memory where to write size.
                let (dest_ptr, dest_size, bytes_written_ptr) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.read_host_buffer,
                    [dest_ptr, dest_size, bytes_written_ptr],
                )?;
                scoped_instrumenter.add_property("dest_size", dest_size);
                let ret = self.read_host_buffer(dest_ptr, dest_size as usize, bytes_written_ptr)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::CreateContractPackageAtHash => {
                // args(0) = pointer to wasm memory where to write 32-byte Hash address
                // args(1) = pointer to wasm memory where to write 32-byte access key address
                // args(2) = boolean flag to determine if the contract can be versioned
                let (hash_dest_ptr, access_dest_ptr, is_locked) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.create_contract_package_at_hash,
                    [hash_dest_ptr, access_dest_ptr],
                )?;
                let package_status = ContractPackageStatus::new(is_locked);
                let (hash_addr, access_addr) =
                    self.create_contract_package_at_hash(package_status)?;

                self.function_address(hash_addr, hash_dest_ptr)?;
                self.function_address(access_addr, access_dest_ptr)?;
                Ok(None)
            }

            FunctionIndex::CreateContractUserGroup => {
                // args(0) = pointer to package key in wasm memory
                // args(1) = size of package key in wasm memory
                // args(2) = pointer to group label in wasm memory
                // args(3) = size of group label in wasm memory
                // args(4) = number of new urefs to generate for the group
                // args(5) = pointer to existing_urefs in wasm memory
                // args(6) = size of existing_urefs in wasm memory
                // args(7) = pointer to location to write size of output (written to host buffer)
                let (
                    package_key_ptr,
                    package_key_size,
                    label_ptr,
                    label_size,
                    num_new_urefs,
                    existing_urefs_ptr,
                    existing_urefs_size,
                    output_size_ptr,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.create_contract_user_group,
                    [
                        package_key_ptr,
                        package_key_size,
                        label_ptr,
                        label_size,
                        num_new_urefs,
                        existing_urefs_ptr,
                        existing_urefs_size,
                        output_size_ptr,
                    ],
                )?;
                scoped_instrumenter
                    .add_property("existing_urefs_size", existing_urefs_size.to_string());
                scoped_instrumenter.add_property("label_size", label_size.to_string());

                let contract_package_hash: ContractPackageHash =
                    self.t_from_mem(package_key_ptr, package_key_size)?;
                let label: String = self.t_from_mem(label_ptr, label_size)?;
                let existing_urefs: BTreeSet<URef> =
                    self.t_from_mem(existing_urefs_ptr, existing_urefs_size)?;

                let ret = self.create_contract_user_group(
                    contract_package_hash,
                    label,
                    num_new_urefs,
                    existing_urefs,
                    output_size_ptr,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::AddContractVersion => {
                // args(0) = pointer to package key in wasm memory
                // args(1) = size of package key in wasm memory
                // args(2) = pointer to entrypoints in wasm memory
                // args(3) = size of entrypoints in wasm memory
                // args(4) = pointer to named keys in wasm memory
                // args(5) = size of named keys in wasm memory
                // args(6) = pointer to output buffer for serialized key
                // args(7) = size of output buffer
                // args(8) = pointer to bytes written
                let (
                    contract_package_hash_ptr,
                    contract_package_hash_size,
                    version_ptr,
                    entry_points_ptr,
                    entry_points_size,
                    named_keys_ptr,
                    named_keys_size,
                    output_ptr,
                    output_size,
                    bytes_written_ptr,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.add_contract_version,
                    [
                        contract_package_hash_ptr,
                        contract_package_hash_size,
                        version_ptr,
                        entry_points_ptr,
                        entry_points_size,
                        named_keys_ptr,
                        named_keys_size,
                        output_ptr,
                        output_size,
                        bytes_written_ptr,
                    ],
                )?;
                scoped_instrumenter
                    .add_property("entry_points_size", entry_points_size.to_string());
                scoped_instrumenter.add_property("named_keys_size", named_keys_size.to_string());

                let contract_package_hash: ContractPackageHash =
                    self.t_from_mem(contract_package_hash_ptr, contract_package_hash_size)?;
                let entry_points: EntryPoints =
                    self.t_from_mem(entry_points_ptr, entry_points_size)?;
                let named_keys: NamedKeys = self.t_from_mem(named_keys_ptr, named_keys_size)?;
                let ret = self.add_contract_version(
                    contract_package_hash,
                    entry_points,
                    named_keys,
                    output_ptr,
                    output_size as usize,
                    bytes_written_ptr,
                    version_ptr,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::DisableContractVersion => {
                // args(0) = pointer to package hash in wasm memory
                // args(1) = size of package hash in wasm memory
                // args(2) = pointer to contract hash in wasm memory
                // args(3) = size of contract hash in wasm memory
                let (package_key_ptr, package_key_size, contract_hash_ptr, contract_hash_size) =
                    Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.disable_contract_version,
                    [
                        package_key_ptr,
                        package_key_size,
                        contract_hash_ptr,
                        contract_hash_size,
                    ],
                )?;
                let contract_package_hash = self.t_from_mem(package_key_ptr, package_key_size)?;
                let contract_hash = self.t_from_mem(contract_hash_ptr, contract_hash_size)?;

                let result = self.disable_contract_version(contract_package_hash, contract_hash)?;

                Ok(Some(RuntimeValue::I32(api_error::i32_from(result))))
            }

            FunctionIndex::CallContractFuncIndex => {
                // args(0) = pointer to contract hash where contract is at in global state
                // args(1) = size of contract hash
                // args(2) = pointer to entry point
                // args(3) = size of entry point
                // args(4) = pointer to function arguments in Wasm memory
                // args(5) = size of arguments
                // args(6) = pointer to result size (output)
                let (
                    contract_hash_ptr,
                    contract_hash_size,
                    entry_point_name_ptr,
                    entry_point_name_size,
                    args_ptr,
                    args_size,
                    result_size_ptr,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.call_contract,
                    [
                        contract_hash_ptr,
                        contract_hash_size,
                        entry_point_name_ptr,
                        entry_point_name_size,
                        args_ptr,
                        args_size,
                        result_size_ptr,
                    ],
                )?;
                scoped_instrumenter
                    .add_property("entry_point_name_size", entry_point_name_size.to_string());
                scoped_instrumenter.add_property("args_size", args_size.to_string());

                let contract_hash: ContractHash =
                    self.t_from_mem(contract_hash_ptr, contract_hash_size)?;
                let entry_point_name: String =
                    self.t_from_mem(entry_point_name_ptr, entry_point_name_size)?;
                let args_bytes: Vec<u8> = {
                    let args_size: u32 = args_size;
                    self.bytes_from_mem(args_ptr, args_size as usize)?
                };

                let ret = self.call_contract_host_buffer(
                    contract_hash,
                    &entry_point_name,
                    args_bytes,
                    result_size_ptr,
                    &mut scoped_instrumenter,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::CallVersionedContract => {
                // args(0) = pointer to contract_package_hash where contract is at in global state
                // args(1) = size of contract_package_hash
                // args(2) = pointer to contract version in wasm memory
                // args(3) = size of contract version in wasm memory
                // args(4) = pointer to method name in wasm memory
                // args(5) = size of method name in wasm memory
                // args(6) = pointer to function arguments in Wasm memory
                // args(7) = size of arguments
                // args(8) = pointer to result size (output)
                let (
                    contract_package_hash_ptr,
                    contract_package_hash_size,
                    contract_version_ptr,
                    contract_package_size,
                    entry_point_name_ptr,
                    entry_point_name_size,
                    args_ptr,
                    args_size,
                    result_size_ptr,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.call_versioned_contract,
                    [
                        contract_package_hash_ptr,
                        contract_package_hash_size,
                        contract_version_ptr,
                        contract_package_size,
                        entry_point_name_ptr,
                        entry_point_name_size,
                        args_ptr,
                        args_size,
                        result_size_ptr,
                    ],
                )?;
                scoped_instrumenter
                    .add_property("entry_point_name_size", entry_point_name_size.to_string());
                scoped_instrumenter.add_property("args_size", args_size.to_string());

                let contract_package_hash: ContractPackageHash =
                    self.t_from_mem(contract_package_hash_ptr, contract_package_hash_size)?;
                let contract_version: Option<ContractVersion> =
                    self.t_from_mem(contract_version_ptr, contract_package_size)?;
                let entry_point_name: String =
                    self.t_from_mem(entry_point_name_ptr, entry_point_name_size)?;
                let args_bytes: Vec<u8> = {
                    let args_size: u32 = args_size;
                    self.bytes_from_mem(args_ptr, args_size as usize)?
                };

                let ret = self.call_versioned_contract_host_buffer(
                    contract_package_hash,
                    contract_version,
                    entry_point_name,
                    args_bytes,
                    result_size_ptr,
                    &mut scoped_instrumenter,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            #[cfg(feature = "test-support")]
            FunctionIndex::PrintIndex => {
                let (text_ptr, text_size) = Args::parse(args)?;
                self.charge_host_function_call(&host_function_costs.print, [text_ptr, text_size])?;
                scoped_instrumenter.add_property("text_size", text_size);
                self.print(text_ptr, text_size)?;
                Ok(None)
            }

            FunctionIndex::GetRuntimeArgsizeIndex => {
                // args(0) = pointer to name of host runtime arg to load
                // args(1) = size of name of the host runtime arg
                // args(2) = pointer to a argument size (output)
                let (name_ptr, name_size, size_ptr) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.get_named_arg_size,
                    [name_ptr, name_size, size_ptr],
                )?;
                scoped_instrumenter.add_property("name_size", name_size.to_string());
                let ret = self.get_named_arg_size(name_ptr, name_size as usize, size_ptr)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::GetRuntimeArgIndex => {
                // args(0) = pointer to serialized argument name
                // args(1) = size of serialized argument name
                // args(2) = pointer to output pointer where host will write argument bytes
                // args(3) = size of available data under output pointer
                let (name_ptr, name_size, dest_ptr, dest_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.get_named_arg,
                    [name_ptr, name_size, dest_ptr, dest_size],
                )?;
                scoped_instrumenter.add_property("name_size", name_size.to_string());
                scoped_instrumenter.add_property("dest_size", dest_size.to_string());
                let ret =
                    self.get_named_arg(name_ptr, name_size as usize, dest_ptr, dest_size as usize)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::RemoveContractUserGroupIndex => {
                // args(0) = pointer to package key in wasm memory
                // args(1) = size of package key in wasm memory
                // args(2) = pointer to serialized group label
                // args(3) = size of serialized group label
                let (package_key_ptr, package_key_size, label_ptr, label_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.remove_contract_user_group,
                    [package_key_ptr, package_key_size, label_ptr, label_size],
                )?;
                scoped_instrumenter.add_property("label_size", label_size.to_string());
                let package_key = self.t_from_mem(package_key_ptr, package_key_size)?;
                let label: Group = self.t_from_mem(label_ptr, label_size)?;

                let ret = self.remove_contract_user_group(package_key, label)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::ExtendContractUserGroupURefsIndex => {
                // args(0) = pointer to package key in wasm memory
                // args(1) = size of package key in wasm memory
                // args(2) = pointer to label name
                // args(3) = label size bytes
                // args(4) = output of size value of host bytes data
                let (package_ptr, package_size, label_ptr, label_size, value_size_ptr) =
                    Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.provision_contract_user_group_uref,
                    [
                        package_ptr,
                        package_size,
                        label_ptr,
                        label_size,
                        value_size_ptr,
                    ],
                )?;
                scoped_instrumenter.add_property("label_size", label_size.to_string());
                let ret = self.provision_contract_user_group_uref(
                    package_ptr,
                    package_size,
                    label_ptr,
                    label_size,
                    value_size_ptr,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::RemoveContractUserGroupURefsIndex => {
                // args(0) = pointer to package key in wasm memory
                // args(1) = size of package key in wasm memory
                // args(2) = pointer to label name
                // args(3) = label size bytes
                // args(4) = pointer to urefs
                // args(5) = size of urefs pointer
                let (package_ptr, package_size, label_ptr, label_size, urefs_ptr, urefs_size) =
                    Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.remove_contract_user_group_urefs,
                    [
                        package_ptr,
                        package_size,
                        label_ptr,
                        label_size,
                        urefs_ptr,
                        urefs_size,
                    ],
                )?;
                scoped_instrumenter.add_property("label_size", label_size.to_string());
                scoped_instrumenter.add_property("urefs_size", urefs_size.to_string());
                let ret = self.remove_contract_user_group_urefs(
                    package_ptr,
                    package_size,
                    label_ptr,
                    label_size,
                    urefs_ptr,
                    urefs_size,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }

            FunctionIndex::Blake2b => {
                let (in_ptr, in_size, out_ptr, out_size) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.blake2b,
                    [in_ptr, in_size, out_ptr, out_size],
                )?;
                scoped_instrumenter.add_property("in_size", in_size.to_string());
                scoped_instrumenter.add_property("out_size", out_size.to_string());
                let input: Vec<u8> = self.bytes_from_mem(in_ptr, in_size as usize)?;
                let digest = account::blake2b(&input);
                if digest.len() != out_size as usize {
                    let err_value = u32::from(api_error::ApiError::BufferTooSmall) as i32;
                    return Ok(Some(RuntimeValue::I32(err_value)));
                }
                self.memory
                    .set(out_ptr, &digest)
                    .map_err(|error| Error::Interpreter(error.into()))?;
                Ok(Some(RuntimeValue::I32(0)))
            }

            FunctionIndex::RecordTransfer => {
                // RecordTransfer is a special cased internal host function only callable by the
                // mint contract and for accounting purposes it isn't represented in protocol data.
                let (
                    maybe_to_ptr,
                    maybe_to_size,
                    source_ptr,
                    source_size,
                    target_ptr,
                    target_size,
                    amount_ptr,
                    amount_size,
                    id_ptr,
                    id_size,
                ): (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) = Args::parse(args)?;
                scoped_instrumenter.add_property("maybe_to_size", maybe_to_size.to_string());
                scoped_instrumenter.add_property("source_size", source_size.to_string());
                scoped_instrumenter.add_property("target_size", target_size.to_string());
                scoped_instrumenter.add_property("amount_size", amount_size.to_string());
                scoped_instrumenter.add_property("id_size", id_size.to_string());
                let maybe_to: Option<AccountHash> = self.t_from_mem(maybe_to_ptr, maybe_to_size)?;
                let source: URef = self.t_from_mem(source_ptr, source_size)?;
                let target: URef = self.t_from_mem(target_ptr, target_size)?;
                let amount: U512 = self.t_from_mem(amount_ptr, amount_size)?;
                let id: Option<u64> = self.t_from_mem(id_ptr, id_size)?;
                self.record_transfer(maybe_to, source, target, amount, id)?;
                Ok(Some(RuntimeValue::I32(0)))
            }

            FunctionIndex::RecordEraInfo => {
                // RecordEraInfo is a special cased internal host function only callable by the
                // auction contract and for accounting purposes it isn't represented in protocol
                // data.
                let (era_id_ptr, era_id_size, era_info_ptr, era_info_size): (u32, u32, u32, u32) =
                    Args::parse(args)?;
                scoped_instrumenter.add_property("era_id_size", era_id_size.to_string());
                scoped_instrumenter.add_property("era_info_size", era_info_size.to_string());
                let era_id: EraId = self.t_from_mem(era_id_ptr, era_id_size)?;
                let era_info: EraInfo = self.t_from_mem(era_info_ptr, era_info_size)?;
                self.record_era_info(era_id, era_info)?;
                Ok(Some(RuntimeValue::I32(0)))
            }
            FunctionIndex::NewDictionaryFuncIndex => {
                // args(0) = pointer to output size (output param)
                let (output_size_ptr,): (u32,) = Args::parse(args)?;

                self.charge_host_function_call(
                    &DEFAULT_HOST_FUNCTION_NEW_DICTIONARY,
                    [output_size_ptr],
                )?;
                let ret = self.new_dictionary(output_size_ptr)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }
            FunctionIndex::DictionaryGetFuncIndex => {
                // args(0) = pointer to uref in Wasm memory
                // args(1) = size of uref in Wasm memory
                // args(2) = pointer to key bytes pointer in Wasm memory
                // args(3) = pointer to key bytes size in Wasm memory
                // args(4) = pointer to output size (output param)
                let (uref_ptr, uref_size, key_bytes_ptr, key_bytes_size, output_size_ptr): (
                    _,
                    u32,
                    _,
                    u32,
                    _,
                ) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.dictionary_get,
                    [key_bytes_ptr, key_bytes_size, output_size_ptr],
                )?;
                scoped_instrumenter.add_property("key_bytes_size", key_bytes_size);
                let ret = self.dictionary_get(
                    uref_ptr,
                    uref_size,
                    key_bytes_ptr,
                    key_bytes_size,
                    output_size_ptr,
                )?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }
            FunctionIndex::DictionaryPutFuncIndex => {
                // args(0) = pointer to uref in Wasm memory
                // args(1) = size of uref in Wasm memory
                // args(2) = pointer to key bytes pointer in Wasm memory
                // args(3) = pointer to key bytes size in Wasm memory
                // args(4) = pointer to value bytes pointer in Wasm memory
                // args(5) = pointer to value bytes size in Wasm memory
                let (uref_ptr, uref_size, key_bytes_ptr, key_bytes_size, value_ptr, value_ptr_size): (_, u32, _, u32, _, u32) = Args::parse(args)?;
                self.charge_host_function_call(
                    &host_function_costs.dictionary_put,
                    [key_bytes_ptr, key_bytes_size, value_ptr, value_ptr_size],
                )?;
                scoped_instrumenter.add_property("key_bytes_size", key_bytes_size);
                scoped_instrumenter.add_property("value_size", value_ptr_size);
                self.dictionary_put(
                    uref_ptr,
                    uref_size,
                    key_bytes_ptr,
                    key_bytes_size,
                    value_ptr,
                    value_ptr_size,
                )?;
                Ok(None)
            }
            FunctionIndex::LoadCallStack => {
                // args(0) (Output) Pointer to number of elements in the call stack.
                // args(1) (Output) Pointer to size in bytes of the serialized call stack.
                let (call_stack_len_ptr, result_size_ptr) = Args::parse(args)?;
                // TODO: add cost table entry once we can upgrade safely
                self.charge_host_function_call(
                    &HostFunction::fixed(10_000),
                    [call_stack_len_ptr, result_size_ptr],
                )?;
                let ret = self.load_call_stack(call_stack_len_ptr, result_size_ptr)?;
                Ok(Some(RuntimeValue::I32(api_error::i32_from(ret))))
            }
        }
    }
}
