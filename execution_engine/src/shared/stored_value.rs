use std::{convert::TryFrom, fmt::Debug};

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};
use serde_bytes::ByteBuf;

use casper_types::{
    bytesrepr::{self, FromBytes, ToBytes, U8_SERIALIZED_LENGTH},
    contracts::ContractPackage,
    system::auction::{Bid, EraInfo, UnbondingPurse},
    CLValue, Contract, ContractWasm, DeployInfo, Transfer,
};

use crate::shared::{account::Account, TypeMismatch};

#[derive(FromPrimitive, ToPrimitive)]
#[repr(u8)]
enum StoredValueTag {
    CLValue = 0,
    Account = 1,
    ContractWasm = 2,
    Contract = 3,
    ContractPackage = 4,
    Transfer = 5,
    DeployInfo = 6,
    EraInfo = 7,
    Bid = 8,
    Withdraw = 9,
}

impl From<StoredValueTag> for u8 {
    fn from(tag: StoredValueTag) -> Self {
        tag.to_u8().expect("StoredValueTag is represented as u8")
    }
}

impl FromBytes for StoredValueTag {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag_value, rem) = FromBytes::from_bytes(bytes)?;
        let tag = StoredValueTag::from_u8(tag_value).ok_or(bytesrepr::Error::Formatting)?;
        Ok((tag, rem))
    }
}

#[derive(Eq, PartialEq, Clone, Debug)]
pub enum StoredValue {
    CLValue(CLValue),
    Account(Account),
    ContractWasm(ContractWasm),
    Contract(Contract),
    ContractPackage(ContractPackage),
    Transfer(Transfer),
    DeployInfo(DeployInfo),
    EraInfo(EraInfo),
    Bid(Box<Bid>),
    Withdraw(Vec<UnbondingPurse>),
}

impl StoredValue {
    pub fn as_cl_value(&self) -> Option<&CLValue> {
        match self {
            StoredValue::CLValue(cl_value) => Some(cl_value),
            _ => None,
        }
    }

    pub fn as_account(&self) -> Option<&Account> {
        match self {
            StoredValue::Account(account) => Some(account),
            _ => None,
        }
    }

    pub fn as_contract(&self) -> Option<&Contract> {
        match self {
            StoredValue::Contract(contract) => Some(contract),
            _ => None,
        }
    }

    pub fn as_contract_wasm(&self) -> Option<&ContractWasm> {
        match self {
            StoredValue::ContractWasm(contract_wasm) => Some(contract_wasm),
            _ => None,
        }
    }

    pub fn as_contract_package(&self) -> Option<&ContractPackage> {
        match self {
            StoredValue::ContractPackage(contract_package) => Some(&contract_package),
            _ => None,
        }
    }

    pub fn as_deploy_info(&self) -> Option<&DeployInfo> {
        match self {
            StoredValue::DeployInfo(deploy_info) => Some(deploy_info),
            _ => None,
        }
    }

    pub fn as_era_info(&self) -> Option<&EraInfo> {
        match self {
            StoredValue::EraInfo(era_info) => Some(era_info),
            _ => None,
        }
    }

    pub fn as_bid(&self) -> Option<&Bid> {
        match self {
            StoredValue::Bid(bid) => Some(bid),
            _ => None,
        }
    }

    pub fn as_withdraw(&self) -> Option<&Vec<UnbondingPurse>> {
        match self {
            StoredValue::Withdraw(unbonding_purses) => Some(unbonding_purses),
            _ => None,
        }
    }

    pub fn type_name(&self) -> String {
        match self {
            StoredValue::CLValue(cl_value) => format!("{:?}", cl_value.cl_type()),
            StoredValue::Account(_) => "Account".to_string(),
            StoredValue::ContractWasm(_) => "Contract".to_string(),
            StoredValue::Contract(_) => "Contract".to_string(),
            StoredValue::ContractPackage(_) => "ContractPackage".to_string(),
            StoredValue::Transfer(_) => "Transfer".to_string(),
            StoredValue::DeployInfo(_) => "DeployInfo".to_string(),
            StoredValue::EraInfo(_) => "EraInfo".to_string(),
            StoredValue::Bid(_) => "Bid".to_string(),
            StoredValue::Withdraw(_) => "Withdraw".to_string(),
        }
    }
}

impl From<CLValue> for StoredValue {
    fn from(value: CLValue) -> StoredValue {
        StoredValue::CLValue(value)
    }
}
impl From<Account> for StoredValue {
    fn from(value: Account) -> StoredValue {
        StoredValue::Account(value)
    }
}
impl From<ContractWasm> for StoredValue {
    fn from(value: ContractWasm) -> StoredValue {
        StoredValue::ContractWasm(value)
    }
}
impl From<Contract> for StoredValue {
    fn from(value: Contract) -> StoredValue {
        StoredValue::Contract(value)
    }
}
impl From<ContractPackage> for StoredValue {
    fn from(value: ContractPackage) -> StoredValue {
        StoredValue::ContractPackage(value)
    }
}
impl From<Bid> for StoredValue {
    fn from(bid: Bid) -> StoredValue {
        StoredValue::Bid(Box::new(bid))
    }
}

impl TryFrom<StoredValue> for CLValue {
    type Error = TypeMismatch;

    fn try_from(stored_value: StoredValue) -> Result<Self, Self::Error> {
        match stored_value {
            StoredValue::CLValue(cl_value) => Ok(cl_value),
            _ => Err(TypeMismatch::new(
                "CLValue".to_string(),
                stored_value.type_name(),
            )),
        }
    }
}

impl TryFrom<StoredValue> for Account {
    type Error = TypeMismatch;

    fn try_from(stored_value: StoredValue) -> Result<Self, Self::Error> {
        match stored_value {
            StoredValue::Account(account) => Ok(account),
            _ => Err(TypeMismatch::new(
                "Account".to_string(),
                stored_value.type_name(),
            )),
        }
    }
}

impl TryFrom<StoredValue> for ContractWasm {
    type Error = TypeMismatch;

    fn try_from(stored_value: StoredValue) -> Result<Self, Self::Error> {
        match stored_value {
            StoredValue::ContractWasm(contract_wasm) => Ok(contract_wasm),
            _ => Err(TypeMismatch::new(
                "ContractWasm".to_string(),
                stored_value.type_name(),
            )),
        }
    }
}

impl TryFrom<StoredValue> for ContractPackage {
    type Error = TypeMismatch;

    fn try_from(stored_value: StoredValue) -> Result<Self, Self::Error> {
        match stored_value {
            StoredValue::ContractPackage(contract_package) => Ok(contract_package),
            _ => Err(TypeMismatch::new(
                "ContractPackage".to_string(),
                stored_value.type_name(),
            )),
        }
    }
}

impl TryFrom<StoredValue> for Contract {
    type Error = TypeMismatch;

    fn try_from(stored_value: StoredValue) -> Result<Self, Self::Error> {
        match stored_value {
            StoredValue::Contract(contract) => Ok(contract),
            _ => Err(TypeMismatch::new(
                "Contract".to_string(),
                stored_value.type_name(),
            )),
        }
    }
}

impl TryFrom<StoredValue> for Transfer {
    type Error = TypeMismatch;

    fn try_from(value: StoredValue) -> Result<Self, Self::Error> {
        match value {
            StoredValue::Transfer(transfer) => Ok(transfer),
            _ => Err(TypeMismatch::new("Transfer".to_string(), value.type_name())),
        }
    }
}

impl TryFrom<StoredValue> for DeployInfo {
    type Error = TypeMismatch;

    fn try_from(value: StoredValue) -> Result<Self, Self::Error> {
        match value {
            StoredValue::DeployInfo(deploy_info) => Ok(deploy_info),
            _ => Err(TypeMismatch::new(
                "DeployInfo".to_string(),
                value.type_name(),
            )),
        }
    }
}

impl TryFrom<StoredValue> for EraInfo {
    type Error = TypeMismatch;

    fn try_from(value: StoredValue) -> Result<Self, Self::Error> {
        match value {
            StoredValue::EraInfo(era_info) => Ok(era_info),
            _ => Err(TypeMismatch::new("EraInfo".to_string(), value.type_name())),
        }
    }
}

impl ToBytes for StoredValue {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        let (tag, mut serialized_data) = match self {
            StoredValue::CLValue(cl_value) => (StoredValueTag::CLValue, cl_value.to_bytes()?),
            StoredValue::Account(account) => (StoredValueTag::Account, account.to_bytes()?),
            StoredValue::ContractWasm(contract_wasm) => {
                (StoredValueTag::ContractWasm, contract_wasm.to_bytes()?)
            }
            StoredValue::Contract(contract_header) => {
                (StoredValueTag::Contract, contract_header.to_bytes()?)
            }
            StoredValue::ContractPackage(contract_package) => (
                StoredValueTag::ContractPackage,
                contract_package.to_bytes()?,
            ),
            StoredValue::Transfer(transfer) => (StoredValueTag::Transfer, transfer.to_bytes()?),
            StoredValue::DeployInfo(deploy_info) => {
                (StoredValueTag::DeployInfo, deploy_info.to_bytes()?)
            }
            StoredValue::EraInfo(era_info) => (StoredValueTag::EraInfo, era_info.to_bytes()?),
            StoredValue::Bid(bid) => (StoredValueTag::Bid, bid.to_bytes()?),
            StoredValue::Withdraw(unbonding_purses) => {
                (StoredValueTag::Withdraw, unbonding_purses.to_bytes()?)
            }
        };
        result.push(tag.into());
        result.append(&mut serialized_data);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
            + match self {
                StoredValue::CLValue(cl_value) => cl_value.serialized_length(),
                StoredValue::Account(account) => account.serialized_length(),
                StoredValue::ContractWasm(contract_wasm) => contract_wasm.serialized_length(),
                StoredValue::Contract(contract_header) => contract_header.serialized_length(),
                StoredValue::ContractPackage(contract_package) => {
                    contract_package.serialized_length()
                }
                StoredValue::Transfer(transfer) => transfer.serialized_length(),
                StoredValue::DeployInfo(deploy_info) => deploy_info.serialized_length(),
                StoredValue::EraInfo(era_info) => era_info.serialized_length(),
                StoredValue::Bid(bid) => bid.serialized_length(),
                StoredValue::Withdraw(unbonding_purses) => unbonding_purses.serialized_length(),
            }
    }
}

impl FromBytes for StoredValue {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = StoredValueTag::from_bytes(bytes)?;
        match tag {
            StoredValueTag::CLValue => CLValue::from_bytes(remainder)
                .map(|(cl_value, remainder)| (StoredValue::CLValue(cl_value), remainder)),
            StoredValueTag::Account => Account::from_bytes(remainder)
                .map(|(account, remainder)| (StoredValue::Account(account), remainder)),
            StoredValueTag::ContractWasm => {
                ContractWasm::from_bytes(remainder).map(|(contract_wasm, remainder)| {
                    (StoredValue::ContractWasm(contract_wasm), remainder)
                })
            }
            StoredValueTag::ContractPackage => {
                ContractPackage::from_bytes(remainder).map(|(contract_package, remainder)| {
                    (StoredValue::ContractPackage(contract_package), remainder)
                })
            }
            StoredValueTag::Contract => Contract::from_bytes(remainder)
                .map(|(contract, remainder)| (StoredValue::Contract(contract), remainder)),
            StoredValueTag::Transfer => Transfer::from_bytes(remainder)
                .map(|(transfer, remainder)| (StoredValue::Transfer(transfer), remainder)),
            StoredValueTag::DeployInfo => DeployInfo::from_bytes(remainder)
                .map(|(deploy_info, remainder)| (StoredValue::DeployInfo(deploy_info), remainder)),
            StoredValueTag::EraInfo => EraInfo::from_bytes(remainder)
                .map(|(deploy_info, remainder)| (StoredValue::EraInfo(deploy_info), remainder)),
            StoredValueTag::Bid => Bid::from_bytes(remainder)
                .map(|(bid, remainder)| (StoredValue::Bid(Box::new(bid)), remainder)),
            StoredValueTag::Withdraw => {
                Vec::<UnbondingPurse>::from_bytes(remainder).map(|(unbonding_purses, remainder)| {
                    (StoredValue::Withdraw(unbonding_purses), remainder)
                })
            }
        }
    }
}

impl Serialize for StoredValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // The JSON representation of a StoredValue is just its bytesrepr
        // While this makes it harder to inspect, it makes deterministic representation simple.
        let bytes = self
            .to_bytes()
            .map_err(|error| ser::Error::custom(format!("{:?}", error)))?;
        ByteBuf::from(bytes).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StoredValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = ByteBuf::deserialize(deserializer)?.into_vec();
        Ok(bytesrepr::deserialize::<StoredValue>(bytes)
            .map_err(|error| de::Error::custom(format!("{:?}", error)))?)
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::prelude::*;

    use casper_types::{
        gens::{
            cl_value_arb, contract_arb, contract_package_arb, contract_wasm_arb, deploy_info_arb,
            transfer_arb,
        },
        system::auction::gens::era_info_arb,
    };

    use super::StoredValue;
    use crate::shared::account::gens::account_arb;

    pub fn stored_value_arb() -> impl Strategy<Value = StoredValue> {
        prop_oneof![
            cl_value_arb().prop_map(StoredValue::CLValue),
            account_arb().prop_map(StoredValue::Account),
            contract_package_arb().prop_map(StoredValue::ContractPackage),
            contract_arb().prop_map(StoredValue::Contract),
            contract_wasm_arb().prop_map(StoredValue::ContractWasm),
            era_info_arb(1..10).prop_map(StoredValue::EraInfo),
            deploy_info_arb().prop_map(StoredValue::DeployInfo),
            transfer_arb().prop_map(StoredValue::Transfer)
        ]
    }
}

#[cfg(test)]
mod tests {
    use proptest::proptest;

    use super::*;

    proptest! {
        #[test]
        fn serialization_roundtrip(v in gens::stored_value_arb()) {
            bytesrepr::test_serialization_roundtrip(&v);
        }
    }
}
