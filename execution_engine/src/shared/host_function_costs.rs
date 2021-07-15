use std::convert::{TryFrom, TryInto};

use datasize::DataSize;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{self, FromBytes, StructReader, StructWriter, ToBytes};

use super::gas::Gas;

/// Representation of argument's cost.
pub type Cost = u32;

/// An identifier that represents an unused argument.
const NOT_USED: Cost = 0;

/// An arbitrary default fixed cost for host functions that were not researched yet.
const DEFAULT_FIXED_COST: Cost = 200;

const DEFAULT_ADD_ASSOCIATED_KEY_COST: u32 = 9_000;
const DEFAULT_ADD_COST: u32 = 5_800;

const DEFAULT_CALL_CONTRACT_COST: u32 = 4_500;
const DEFAULT_CALL_CONTRACT_ARGS_SIZE_WEIGHT: u32 = 420;

const DEFAULT_CREATE_PURSE_COST: u32 = 170_000;
const DEFAULT_GET_BALANCE_COST: u32 = 3_800;
const DEFAULT_GET_BLOCKTIME_COST: u32 = 330;
const DEFAULT_GET_CALLER_COST: u32 = 380;
const DEFAULT_GET_KEY_COST: u32 = 2_000;
const DEFAULT_GET_KEY_NAME_SIZE_WEIGHT: u32 = 440;
const DEFAULT_GET_MAIN_PURSE_COST: u32 = 1_300;
const DEFAULT_GET_PHASE_COST: u32 = 710;
const DEFAULT_GET_SYSTEM_CONTRACT_COST: u32 = 1_100;
const DEFAULT_HAS_KEY_COST: u32 = 1_500;
const DEFAULT_HAS_KEY_NAME_SIZE_WEIGHT: u32 = 840;
const DEFAULT_IS_VALID_UREF_COST: u32 = 760;
const DEFAULT_LOAD_NAMED_KEYS_COST: u32 = 42_000;
const DEFAULT_NEW_UREF_COST: u32 = 17_000;
const DEFAULT_NEW_UREF_VALUE_SIZE_WEIGHT: u32 = 590;

const DEFAULT_PRINT_COST: u32 = 20_000;
const DEFAULT_PRINT_TEXT_SIZE_WEIGHT: u32 = 4_600;

const DEFAULT_PUT_KEY_COST: u32 = 38_000;
const DEFAULT_PUT_KEY_NAME_SIZE_WEIGHT: u32 = 1_100;

const DEFAULT_READ_HOST_BUFFER_COST: u32 = 3_500;
const DEFAULT_READ_HOST_BUFFER_DEST_SIZE_WEIGHT: u32 = 310;

const DEFAULT_READ_VALUE_COST: u32 = 6_000;
const DEFAULT_DICTIONARY_GET_COST: u32 = 5_500;
const DEFAULT_DICTIONARY_GET_KEY_SIZE_WEIGHT: u32 = 590;

const DEFAULT_REMOVE_ASSOCIATED_KEY_COST: u32 = 4_200;

const DEFAULT_REMOVE_KEY_COST: u32 = 61_000;
const DEFAULT_REMOVE_KEY_NAME_SIZE_WEIGHT: u32 = 3_200;

const DEFAULT_RET_COST: u32 = 23_000;
const DEFAULT_RET_VALUE_SIZE_WEIGHT: u32 = 420;

const DEFAULT_REVERT_COST: u32 = 500;
const DEFAULT_SET_ACTION_THRESHOLD_COST: u32 = 74_000;
const DEFAULT_TRANSFER_FROM_PURSE_TO_ACCOUNT_COST: u32 = 160_000;
const DEFAULT_TRANSFER_FROM_PURSE_TO_PURSE_COST: u32 = 82_000;
const DEFAULT_TRANSFER_TO_ACCOUNT_COST: u32 = 24_000;
const DEFAULT_UPDATE_ASSOCIATED_KEY_COST: u32 = 4_200;

const DEFAULT_WRITE_COST: u32 = 14_000;
const DEFAULT_WRITE_VALUE_SIZE_WEIGHT: u32 = 980;

const DEFAULT_DICTIONARY_PUT_COST: u32 = 9_500;
const DEFAULT_DICTIONARY_PUT_KEY_BYTES_SIZE_WEIGHT: u32 = 1_800;
const DEFAULT_DICTIONARY_PUT_VALUE_SIZE_WEIGHT: u32 = 520;

const DEFAULT_NEW_DICTIONARY_COST: u32 = DEFAULT_NEW_UREF_COST;

pub(crate) const DEFAULT_HOST_FUNCTION_NEW_DICTIONARY: HostFunction<[Cost; 1]> =
    HostFunction::new(DEFAULT_NEW_DICTIONARY_COST, [NOT_USED]);

/// Representation of a host function cost
///
/// Total gas cost is equal to `cost` + sum of each argument weight multiplied by the byte size of
/// the data.
#[derive(Copy, Clone, PartialEq, Eq, Deserialize, Serialize, Debug, DataSize)]
pub struct HostFunction<T> {
    /// How much user is charged for cost only
    cost: Cost,
    arguments: T,
}

impl<T> Default for HostFunction<T>
where
    T: Default,
{
    fn default() -> Self {
        HostFunction::new(DEFAULT_FIXED_COST, Default::default())
    }
}

impl<T> HostFunction<T> {
    pub const fn new(cost: Cost, arguments: T) -> Self {
        Self { cost, arguments }
    }

    pub fn cost(&self) -> Cost {
        self.cost
    }
}

impl<T> HostFunction<T>
where
    T: Default,
{
    pub fn fixed(cost: Cost) -> Self {
        Self {
            cost,
            ..Default::default()
        }
    }
}

impl<T> HostFunction<T>
where
    T: AsRef<[Cost]>,
{
    pub fn arguments(&self) -> &[Cost] {
        self.arguments.as_ref()
    }

    pub fn take_arguments(self) -> T {
        self.arguments
    }

    /// Calculate gas cost for a host function
    pub fn calculate_gas_cost(&self, weights: T) -> Gas {
        let mut gas = Gas::new(self.cost.into());
        for (argument, weight) in self.arguments.as_ref().iter().zip(weights.as_ref()) {
            let lhs = Gas::new((*argument).into());
            let rhs = Gas::new((*weight).into());
            gas += lhs * rhs;
        }
        gas
    }
}

impl<T> Distribution<HostFunction<T>> for Standard
where
    Standard: Distribution<T>,
    T: AsRef<[Cost]>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> HostFunction<T> {
        let cost = rng.gen::<Cost>();
        let arguments = rng.gen();
        HostFunction::new(cost, arguments)
    }
}

impl<T> ToBytes for HostFunction<T>
where
    T: AsRef<[Cost]>,
{
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut ret = bytesrepr::unchecked_allocate_buffer(self);
        ret.append(&mut self.cost.to_bytes()?);

        let length_prefixed_arguments = self.arguments.as_ref().to_vec();
        ret.append(&mut length_prefixed_arguments.to_bytes()?);
        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.cost.serialized_length() + self.arguments.as_ref().to_vec().serialized_length()
    }
}

impl<T> FromBytes for HostFunction<T>
where
    for<'a> T: TryFrom<&'a [Cost]>,
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (cost, bytes) = FromBytes::from_bytes(bytes)?;

        let (length_prefixed_arguments, bytes): (Vec<Cost>, _) = FromBytes::from_bytes(bytes)?;
        let arguments: T = length_prefixed_arguments
            .as_slice()
            .try_into()
            .map_err(|_| bytesrepr::Error::Formatting)?;
        Ok((Self { cost, arguments }, bytes))
    }
}

#[derive(FromPrimitive, ToPrimitive)]
enum HostFunctionCostsKeys {
    ReadValue = 100,
    DictionaryGet = 101,
    Write = 102,
    DictionaryPut = 103,
    Add = 104,
    NewURef = 105,
    LoadNamedKeys = 106,
    Ret = 107,
    GetKey = 108,
    HasKey = 109,
    PutKey = 110,
    RemoveKey = 111,
    Revert = 112,
    IsValidURef = 113,
    AddAssociatedKey = 114,
    RemoveAssociatedKey = 115,
    UpdateAssociatedKey = 116,
    SetActionThreshold = 117,
    GetCaller = 118,
    GetBlocktime = 119,
    CreatePurse = 120,
    TransferToAccount = 121,
    TransferFromPurseToAccount = 122,
    TransferFromPurseToPurse = 123,
    GetBalance = 124,
    GetPhase = 125,
    GetSystemContract = 126,
    GetMainPurse = 127,
    ReadHostBuffer = 128,
    CreateContractPackageAtHash = 129,
    CreateContractUserGroup = 130,
    AddContractVersion = 131,
    DisableContractVersion = 132,
    CallContract = 133,
    CallVersionedContract = 134,
    GetNamedArgSize = 135,
    GetNamedArg = 136,
    RemoveContractUserGroup = 137,
    ProvisionContractUserGroupUref = 138,
    RemoveContractUserGroupUrefs = 139,
    Print = 140,
    Blake2b = 141,
}

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct HostFunctionCosts {
    pub read_value: HostFunction<[Cost; 3]>,
    #[serde(alias = "read_value_local")]
    pub dictionary_get: HostFunction<[Cost; 3]>,
    pub write: HostFunction<[Cost; 4]>,
    #[serde(alias = "write_local")]
    pub dictionary_put: HostFunction<[Cost; 4]>,
    pub add: HostFunction<[Cost; 4]>,
    pub new_uref: HostFunction<[Cost; 3]>,
    pub load_named_keys: HostFunction<[Cost; 2]>,
    pub ret: HostFunction<[Cost; 2]>,
    pub get_key: HostFunction<[Cost; 5]>,
    pub has_key: HostFunction<[Cost; 2]>,
    pub put_key: HostFunction<[Cost; 4]>,
    pub remove_key: HostFunction<[Cost; 2]>,
    pub revert: HostFunction<[Cost; 1]>,
    pub is_valid_uref: HostFunction<[Cost; 2]>,
    pub add_associated_key: HostFunction<[Cost; 3]>,
    pub remove_associated_key: HostFunction<[Cost; 2]>,
    pub update_associated_key: HostFunction<[Cost; 3]>,
    pub set_action_threshold: HostFunction<[Cost; 2]>,
    pub get_caller: HostFunction<[Cost; 1]>,
    pub get_blocktime: HostFunction<[Cost; 1]>,
    pub create_purse: HostFunction<[Cost; 2]>,
    pub transfer_to_account: HostFunction<[Cost; 7]>,
    pub transfer_from_purse_to_account: HostFunction<[Cost; 9]>,
    pub transfer_from_purse_to_purse: HostFunction<[Cost; 8]>,
    pub get_balance: HostFunction<[Cost; 3]>,
    pub get_phase: HostFunction<[Cost; 1]>,
    pub get_system_contract: HostFunction<[Cost; 3]>,
    pub get_main_purse: HostFunction<[Cost; 1]>,
    pub read_host_buffer: HostFunction<[Cost; 3]>,
    pub create_contract_package_at_hash: HostFunction<[Cost; 2]>,
    pub create_contract_user_group: HostFunction<[Cost; 8]>,
    pub add_contract_version: HostFunction<[Cost; 10]>,
    pub disable_contract_version: HostFunction<[Cost; 4]>,
    pub call_contract: HostFunction<[Cost; 7]>,
    pub call_versioned_contract: HostFunction<[Cost; 9]>,
    pub get_named_arg_size: HostFunction<[Cost; 3]>,
    pub get_named_arg: HostFunction<[Cost; 4]>,
    pub remove_contract_user_group: HostFunction<[Cost; 4]>,
    pub provision_contract_user_group_uref: HostFunction<[Cost; 5]>,
    pub remove_contract_user_group_urefs: HostFunction<[Cost; 6]>,
    pub print: HostFunction<[Cost; 2]>,
    pub blake2b: HostFunction<[Cost; 4]>,
}

impl Default for HostFunctionCosts {
    fn default() -> Self {
        Self {
            read_value: HostFunction::fixed(DEFAULT_READ_VALUE_COST),
            dictionary_get: HostFunction::new(
                DEFAULT_DICTIONARY_GET_COST,
                [NOT_USED, DEFAULT_DICTIONARY_GET_KEY_SIZE_WEIGHT, NOT_USED],
            ),
            write: HostFunction::new(
                DEFAULT_WRITE_COST,
                [
                    NOT_USED,
                    NOT_USED,
                    NOT_USED,
                    DEFAULT_WRITE_VALUE_SIZE_WEIGHT,
                ],
            ),
            dictionary_put: HostFunction::new(
                DEFAULT_DICTIONARY_PUT_COST,
                [
                    NOT_USED,
                    DEFAULT_DICTIONARY_PUT_KEY_BYTES_SIZE_WEIGHT,
                    NOT_USED,
                    DEFAULT_DICTIONARY_PUT_VALUE_SIZE_WEIGHT,
                ],
            ),
            add: HostFunction::fixed(DEFAULT_ADD_COST),
            new_uref: HostFunction::new(
                DEFAULT_NEW_UREF_COST,
                [NOT_USED, NOT_USED, DEFAULT_NEW_UREF_VALUE_SIZE_WEIGHT],
            ),
            load_named_keys: HostFunction::fixed(DEFAULT_LOAD_NAMED_KEYS_COST),
            ret: HostFunction::new(DEFAULT_RET_COST, [NOT_USED, DEFAULT_RET_VALUE_SIZE_WEIGHT]),
            get_key: HostFunction::new(
                DEFAULT_GET_KEY_COST,
                [
                    NOT_USED,
                    DEFAULT_GET_KEY_NAME_SIZE_WEIGHT,
                    NOT_USED,
                    NOT_USED,
                    NOT_USED,
                ],
            ),
            has_key: HostFunction::new(
                DEFAULT_HAS_KEY_COST,
                [NOT_USED, DEFAULT_HAS_KEY_NAME_SIZE_WEIGHT],
            ),
            put_key: HostFunction::new(
                DEFAULT_PUT_KEY_COST,
                [
                    NOT_USED,
                    DEFAULT_PUT_KEY_NAME_SIZE_WEIGHT,
                    NOT_USED,
                    NOT_USED,
                ],
            ),
            remove_key: HostFunction::new(
                DEFAULT_REMOVE_KEY_COST,
                [NOT_USED, DEFAULT_REMOVE_KEY_NAME_SIZE_WEIGHT],
            ),
            revert: HostFunction::fixed(DEFAULT_REVERT_COST),
            is_valid_uref: HostFunction::fixed(DEFAULT_IS_VALID_UREF_COST),
            add_associated_key: HostFunction::fixed(DEFAULT_ADD_ASSOCIATED_KEY_COST),
            remove_associated_key: HostFunction::fixed(DEFAULT_REMOVE_ASSOCIATED_KEY_COST),
            update_associated_key: HostFunction::fixed(DEFAULT_UPDATE_ASSOCIATED_KEY_COST),
            set_action_threshold: HostFunction::fixed(DEFAULT_SET_ACTION_THRESHOLD_COST),
            get_caller: HostFunction::fixed(DEFAULT_GET_CALLER_COST),
            get_blocktime: HostFunction::fixed(DEFAULT_GET_BLOCKTIME_COST),
            create_purse: HostFunction::fixed(DEFAULT_CREATE_PURSE_COST),
            transfer_to_account: HostFunction::fixed(DEFAULT_TRANSFER_TO_ACCOUNT_COST),
            transfer_from_purse_to_account: HostFunction::fixed(
                DEFAULT_TRANSFER_FROM_PURSE_TO_ACCOUNT_COST,
            ),
            transfer_from_purse_to_purse: HostFunction::fixed(
                DEFAULT_TRANSFER_FROM_PURSE_TO_PURSE_COST,
            ),
            get_balance: HostFunction::fixed(DEFAULT_GET_BALANCE_COST),
            get_phase: HostFunction::fixed(DEFAULT_GET_PHASE_COST),
            get_system_contract: HostFunction::fixed(DEFAULT_GET_SYSTEM_CONTRACT_COST),
            get_main_purse: HostFunction::fixed(DEFAULT_GET_MAIN_PURSE_COST),
            read_host_buffer: HostFunction::new(
                DEFAULT_READ_HOST_BUFFER_COST,
                [
                    NOT_USED,
                    DEFAULT_READ_HOST_BUFFER_DEST_SIZE_WEIGHT,
                    NOT_USED,
                ],
            ),
            create_contract_package_at_hash: HostFunction::default(),
            create_contract_user_group: HostFunction::default(),
            add_contract_version: HostFunction::default(),
            disable_contract_version: HostFunction::default(),
            call_contract: HostFunction::new(
                DEFAULT_CALL_CONTRACT_COST,
                [
                    NOT_USED,
                    NOT_USED,
                    NOT_USED,
                    NOT_USED,
                    NOT_USED,
                    DEFAULT_CALL_CONTRACT_ARGS_SIZE_WEIGHT,
                    NOT_USED,
                ],
            ),
            call_versioned_contract: HostFunction::default(),
            get_named_arg_size: HostFunction::default(),
            get_named_arg: HostFunction::default(),
            remove_contract_user_group: HostFunction::default(),
            provision_contract_user_group_uref: HostFunction::default(),
            remove_contract_user_group_urefs: HostFunction::default(),
            print: HostFunction::new(
                DEFAULT_PRINT_COST,
                [NOT_USED, DEFAULT_PRINT_TEXT_SIZE_WEIGHT],
            ),
            blake2b: HostFunction::default(),
        }
    }
}

impl ToBytes for HostFunctionCosts {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut writer = StructWriter::new();

        writer.write_pair(HostFunctionCostsKeys::ReadValue, self.read_value)?;
        writer.write_pair(HostFunctionCostsKeys::DictionaryGet, self.dictionary_get)?;
        writer.write_pair(HostFunctionCostsKeys::Write, self.write)?;
        writer.write_pair(HostFunctionCostsKeys::DictionaryPut, self.dictionary_put)?;
        writer.write_pair(HostFunctionCostsKeys::Add, self.add)?;
        writer.write_pair(HostFunctionCostsKeys::NewURef, self.new_uref)?;
        writer.write_pair(HostFunctionCostsKeys::LoadNamedKeys, self.load_named_keys)?;
        writer.write_pair(HostFunctionCostsKeys::Ret, self.ret)?;
        writer.write_pair(HostFunctionCostsKeys::GetKey, self.get_key)?;
        writer.write_pair(HostFunctionCostsKeys::HasKey, self.has_key)?;
        writer.write_pair(HostFunctionCostsKeys::PutKey, self.put_key)?;
        writer.write_pair(HostFunctionCostsKeys::RemoveKey, self.remove_key)?;
        writer.write_pair(HostFunctionCostsKeys::Revert, self.revert)?;
        writer.write_pair(HostFunctionCostsKeys::IsValidURef, self.is_valid_uref)?;
        writer.write_pair(
            HostFunctionCostsKeys::AddAssociatedKey,
            self.add_associated_key,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::RemoveAssociatedKey,
            self.remove_associated_key,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::UpdateAssociatedKey,
            self.update_associated_key,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::SetActionThreshold,
            self.set_action_threshold,
        )?;
        writer.write_pair(HostFunctionCostsKeys::GetCaller, self.get_caller)?;
        writer.write_pair(HostFunctionCostsKeys::GetBlocktime, self.get_blocktime)?;
        writer.write_pair(HostFunctionCostsKeys::CreatePurse, self.create_purse)?;
        writer.write_pair(
            HostFunctionCostsKeys::TransferToAccount,
            self.transfer_to_account,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::TransferFromPurseToAccount,
            self.transfer_from_purse_to_account,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::TransferFromPurseToPurse,
            self.transfer_from_purse_to_purse,
        )?;
        writer.write_pair(HostFunctionCostsKeys::GetBalance, self.get_balance)?;
        writer.write_pair(HostFunctionCostsKeys::GetPhase, self.get_phase)?;
        writer.write_pair(
            HostFunctionCostsKeys::GetSystemContract,
            self.get_system_contract,
        )?;
        writer.write_pair(HostFunctionCostsKeys::GetMainPurse, self.get_main_purse)?;
        writer.write_pair(HostFunctionCostsKeys::ReadHostBuffer, self.read_host_buffer)?;
        writer.write_pair(
            HostFunctionCostsKeys::CreateContractPackageAtHash,
            self.create_contract_package_at_hash,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::CreateContractUserGroup,
            self.create_contract_user_group,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::AddContractVersion,
            self.add_contract_version,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::DisableContractVersion,
            self.disable_contract_version,
        )?;
        writer.write_pair(HostFunctionCostsKeys::CallContract, self.call_contract)?;
        writer.write_pair(
            HostFunctionCostsKeys::CallVersionedContract,
            self.call_versioned_contract,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::GetNamedArgSize,
            self.get_named_arg_size,
        )?;
        writer.write_pair(HostFunctionCostsKeys::GetNamedArg, self.get_named_arg)?;
        writer.write_pair(
            HostFunctionCostsKeys::RemoveContractUserGroup,
            self.remove_contract_user_group,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::ProvisionContractUserGroupUref,
            self.provision_contract_user_group_uref,
        )?;
        writer.write_pair(
            HostFunctionCostsKeys::RemoveContractUserGroupUrefs,
            self.remove_contract_user_group_urefs,
        )?;
        writer.write_pair(HostFunctionCostsKeys::Print, self.print)?;
        writer.write_pair(HostFunctionCostsKeys::Blake2b, self.blake2b)?;

        writer.finish()
    }

    fn serialized_length(&self) -> usize {
        bytesrepr::serialized_struct_fields_length(&[
            self.read_value.serialized_length(),
            self.dictionary_get.serialized_length(),
            self.write.serialized_length(),
            self.dictionary_put.serialized_length(),
            self.add.serialized_length(),
            self.new_uref.serialized_length(),
            self.load_named_keys.serialized_length(),
            self.ret.serialized_length(),
            self.get_key.serialized_length(),
            self.has_key.serialized_length(),
            self.put_key.serialized_length(),
            self.remove_key.serialized_length(),
            self.revert.serialized_length(),
            self.is_valid_uref.serialized_length(),
            self.add_associated_key.serialized_length(),
            self.remove_associated_key.serialized_length(),
            self.update_associated_key.serialized_length(),
            self.set_action_threshold.serialized_length(),
            self.get_caller.serialized_length(),
            self.get_blocktime.serialized_length(),
            self.create_purse.serialized_length(),
            self.transfer_to_account.serialized_length(),
            self.transfer_from_purse_to_account.serialized_length(),
            self.transfer_from_purse_to_purse.serialized_length(),
            self.get_balance.serialized_length(),
            self.get_phase.serialized_length(),
            self.get_system_contract.serialized_length(),
            self.get_main_purse.serialized_length(),
            self.read_host_buffer.serialized_length(),
            self.create_contract_package_at_hash.serialized_length(),
            self.create_contract_user_group.serialized_length(),
            self.add_contract_version.serialized_length(),
            self.disable_contract_version.serialized_length(),
            self.call_contract.serialized_length(),
            self.call_versioned_contract.serialized_length(),
            self.get_named_arg_size.serialized_length(),
            self.get_named_arg.serialized_length(),
            self.remove_contract_user_group.serialized_length(),
            self.provision_contract_user_group_uref.serialized_length(),
            self.remove_contract_user_group_urefs.serialized_length(),
            self.print.serialized_length(),
            self.blake2b.serialized_length(),
        ])
    }
}

impl FromBytes for HostFunctionCosts {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let mut reader = StructReader::new(bytes);

        let mut host_function_costs = HostFunctionCosts::default();

        while let Some(key) = reader.read_key()? {
            match HostFunctionCostsKeys::from_u64(key) {
                Some(HostFunctionCostsKeys::ReadValue) => {
                    host_function_costs.read_value = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::DictionaryGet) => {
                    host_function_costs.dictionary_get = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::Write) => {
                    host_function_costs.write = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::DictionaryPut) => {
                    host_function_costs.dictionary_put = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::Add) => {
                    host_function_costs.add = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::NewURef) => {
                    host_function_costs.new_uref = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::LoadNamedKeys) => {
                    host_function_costs.load_named_keys = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::Ret) => {
                    host_function_costs.ret = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetKey) => {
                    host_function_costs.get_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::HasKey) => {
                    host_function_costs.has_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::PutKey) => {
                    host_function_costs.put_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::RemoveKey) => {
                    host_function_costs.remove_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::Revert) => {
                    host_function_costs.revert = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::IsValidURef) => {
                    host_function_costs.is_valid_uref = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::AddAssociatedKey) => {
                    host_function_costs.add_associated_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::RemoveAssociatedKey) => {
                    host_function_costs.remove_associated_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::UpdateAssociatedKey) => {
                    host_function_costs.update_associated_key = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::SetActionThreshold) => {
                    host_function_costs.set_action_threshold = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetCaller) => {
                    host_function_costs.get_caller = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetBlocktime) => {
                    host_function_costs.get_blocktime = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::CreatePurse) => {
                    host_function_costs.create_purse = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::TransferToAccount) => {
                    host_function_costs.transfer_to_account = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::TransferFromPurseToAccount) => {
                    host_function_costs.transfer_from_purse_to_account = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::TransferFromPurseToPurse) => {
                    host_function_costs.transfer_from_purse_to_purse = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetBalance) => {
                    host_function_costs.get_balance = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetPhase) => {
                    host_function_costs.get_phase = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetSystemContract) => {
                    host_function_costs.get_system_contract = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetMainPurse) => {
                    host_function_costs.get_main_purse = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::ReadHostBuffer) => {
                    host_function_costs.read_host_buffer = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::CreateContractPackageAtHash) => {
                    host_function_costs.create_contract_package_at_hash = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::CreateContractUserGroup) => {
                    host_function_costs.create_contract_user_group = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::AddContractVersion) => {
                    host_function_costs.add_contract_version = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::DisableContractVersion) => {
                    host_function_costs.disable_contract_version = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::CallContract) => {
                    host_function_costs.call_contract = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::CallVersionedContract) => {
                    host_function_costs.call_versioned_contract = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetNamedArgSize) => {
                    host_function_costs.get_named_arg_size = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::GetNamedArg) => {
                    host_function_costs.get_named_arg = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::RemoveContractUserGroup) => {
                    host_function_costs.remove_contract_user_group = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::ProvisionContractUserGroupUref) => {
                    host_function_costs.provision_contract_user_group_uref = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::RemoveContractUserGroupUrefs) => {
                    host_function_costs.remove_contract_user_group_urefs = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::Print) => {
                    host_function_costs.print = reader.read_value()?
                }
                Some(HostFunctionCostsKeys::Blake2b) => {
                    host_function_costs.blake2b = reader.read_value()?
                }
                None => reader.skip_value()?,
            }
        }

        Ok((host_function_costs, reader.finish()))
    }
}

impl Distribution<HostFunctionCosts> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> HostFunctionCosts {
        HostFunctionCosts {
            read_value: rng.gen(),
            dictionary_get: rng.gen(),
            write: rng.gen(),
            dictionary_put: rng.gen(),
            add: rng.gen(),
            new_uref: rng.gen(),
            load_named_keys: rng.gen(),
            ret: rng.gen(),
            get_key: rng.gen(),
            has_key: rng.gen(),
            put_key: rng.gen(),
            remove_key: rng.gen(),
            revert: rng.gen(),
            is_valid_uref: rng.gen(),
            add_associated_key: rng.gen(),
            remove_associated_key: rng.gen(),
            update_associated_key: rng.gen(),
            set_action_threshold: rng.gen(),
            get_caller: rng.gen(),
            get_blocktime: rng.gen(),
            create_purse: rng.gen(),
            transfer_to_account: rng.gen(),
            transfer_from_purse_to_account: rng.gen(),
            transfer_from_purse_to_purse: rng.gen(),
            get_balance: rng.gen(),
            get_phase: rng.gen(),
            get_system_contract: rng.gen(),
            get_main_purse: rng.gen(),
            read_host_buffer: rng.gen(),
            create_contract_package_at_hash: rng.gen(),
            create_contract_user_group: rng.gen(),
            add_contract_version: rng.gen(),
            disable_contract_version: rng.gen(),
            call_contract: rng.gen(),
            call_versioned_contract: rng.gen(),
            get_named_arg_size: rng.gen(),
            get_named_arg: rng.gen(),
            remove_contract_user_group: rng.gen(),
            provision_contract_user_group_uref: rng.gen(),
            remove_contract_user_group_urefs: rng.gen(),
            print: rng.gen(),
            blake2b: rng.gen(),
        }
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::prelude::*;

    use super::{Cost, HostFunction, HostFunctionCosts};

    pub fn host_function_cost_arb<T: Copy + Arbitrary>() -> impl Strategy<Value = HostFunction<T>> {
        (any::<Cost>(), any::<T>()).prop_map(|(cost, arguments)| HostFunction::new(cost, arguments))
    }

    prop_compose! {
        pub fn host_function_costs_arb() (
            read_value in host_function_cost_arb(),
            dictionary_get in host_function_cost_arb(),
            write in host_function_cost_arb(),
            dictionary_put in host_function_cost_arb(),
            add in host_function_cost_arb(),
            new_uref in host_function_cost_arb(),
            load_named_keys in host_function_cost_arb(),
            ret in host_function_cost_arb(),
            get_key in host_function_cost_arb(),
            has_key in host_function_cost_arb(),
            put_key in host_function_cost_arb(),
            remove_key in host_function_cost_arb(),
            revert in host_function_cost_arb(),
            is_valid_uref in host_function_cost_arb(),
            add_associated_key in host_function_cost_arb(),
            remove_associated_key in host_function_cost_arb(),
            update_associated_key in host_function_cost_arb(),
            set_action_threshold in host_function_cost_arb(),
            get_caller in host_function_cost_arb(),
            get_blocktime in host_function_cost_arb(),
            create_purse in host_function_cost_arb(),
            transfer_to_account in host_function_cost_arb(),
            transfer_from_purse_to_account in host_function_cost_arb(),
            transfer_from_purse_to_purse in host_function_cost_arb(),
            get_balance in host_function_cost_arb(),
            get_phase in host_function_cost_arb(),
            get_system_contract in host_function_cost_arb(),
            get_main_purse in host_function_cost_arb(),
            read_host_buffer in host_function_cost_arb(),
            create_contract_package_at_hash in host_function_cost_arb(),
            create_contract_user_group in host_function_cost_arb(),
            add_contract_version in host_function_cost_arb(),
            disable_contract_version in host_function_cost_arb(),
            call_contract in host_function_cost_arb(),
            call_versioned_contract in host_function_cost_arb(),
            get_named_arg_size in host_function_cost_arb(),
            get_named_arg in host_function_cost_arb(),
            remove_contract_user_group in host_function_cost_arb(),
            provision_contract_user_group_uref in host_function_cost_arb(),
            remove_contract_user_group_urefs in host_function_cost_arb(),
            print in host_function_cost_arb(),
            blake2b in host_function_cost_arb(),
        ) -> HostFunctionCosts {
            HostFunctionCosts {
                read_value,
                dictionary_get,
                write,
                dictionary_put,
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use casper_types::U512;

    use super::*;

    const COST: Cost = 42;
    const ARGUMENT_COSTS: [Cost; 3] = [123, 456, 789];
    const WEIGHTS: [Cost; 3] = [1000, 1100, 1200];

    #[test]
    fn calculate_gas_cost_for_host_function() {
        let host_function = HostFunction::new(COST, ARGUMENT_COSTS);
        let expected_cost = COST
            + (ARGUMENT_COSTS[0] * WEIGHTS[0])
            + (ARGUMENT_COSTS[1] * WEIGHTS[1])
            + (ARGUMENT_COSTS[2] * WEIGHTS[2]);
        assert_eq!(
            host_function.calculate_gas_cost(WEIGHTS),
            Gas::new(expected_cost.into())
        );
    }

    #[test]
    fn calculate_gas_cost_would_overflow() {
        let large_value = Cost::max_value();

        let host_function = HostFunction::new(
            large_value,
            [large_value, large_value, large_value, large_value],
        );

        let lhs =
            host_function.calculate_gas_cost([large_value, large_value, large_value, large_value]);

        let large_value = U512::from(large_value);
        let rhs = large_value + (U512::from(4) * large_value * large_value);

        assert_eq!(lhs, Gas::new(rhs));
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use casper_types::bytesrepr;

    use super::*;

    type Signature = [Cost; 10];

    proptest! {
        #[test]
        fn test_host_function(host_function in gens::host_function_cost_arb::<Signature>()) {
            bytesrepr::test_serialization_roundtrip(&host_function);
        }

        #[test]
        fn test_host_function_costs(host_function_costs in gens::host_function_costs_arb()) {
            bytesrepr::test_serialization_roundtrip(&host_function_costs);
        }
    }
}
