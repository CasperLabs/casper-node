use std::{
    collections::{BTreeMap, BTreeSet},
    iter::FromIterator,
};

use casper_execution_engine::shared::{
    account::{Account, ActionThresholds, AssociatedKeys},
    stored_value::StoredValue,
    wasm,
};
use casper_types::{
    account::{AccountHash, Weight},
    contracts::{ContractPackageStatus, ContractVersions, DisabledVersions, Groups, NamedKeys},
    system::auction::{Bid, EraInfo, SeigniorageAllocation, UnbondingPurse},
    AccessRights, CLType, CLTyped, CLValue, Contract, ContractHash, ContractPackage,
    ContractPackageHash, ContractVersionKey, ContractWasm, ContractWasmHash, DeployHash,
    DeployInfo, EntryPoint, EntryPointAccess, EntryPointType, EntryPoints, EraId, Group, Key,
    NamedKey, Parameter, ProtocolVersion, PublicKey, SecretKey, Transfer, TransferAddr, Transform,
    URef, U128, U256, U512,
};
use casper_validation::{
    abi::{ABIFixture, ABITestCase},
    error::Error,
    Fixture, TestFixtures,
};

pub fn make_abi_test_fixtures() -> Result<TestFixtures, Error> {
    let basic = {
        let mut basic = BTreeMap::new();
        basic.insert(
            "SerializeU8".to_string(),
            ABITestCase::from_inputs(vec![254u8.into()])?,
        );
        basic.insert(
            "SerializeU16".to_string(),
            ABITestCase::from_inputs(vec![62356u16.into()])?,
        );
        basic.insert(
            "SerializeU32".to_string(),
            ABITestCase::from_inputs(vec![3490072870u32.into()])?,
        );
        basic.insert(
            "SerializeU64".to_string(),
            ABITestCase::from_inputs(vec![10829133186225377555u64.into()])?,
        );
        basic.insert(
            "SerializeString".to_string(),
            ABITestCase::from_inputs(vec!["Hello, world!".to_string().into()])?,
        );
        basic.insert(
            "SerializeBool".to_string(),
            ABITestCase::from_inputs(vec![true.into(), false.into()])?,
        );
        Fixture::ABI("basic".to_string(), ABIFixture::from(basic))
    };

    let transfer = Transfer::new(
        DeployHash::new([44; 32]),
        AccountHash::new([100; 32]),
        Some(AccountHash::new([101; 32])),
        URef::new([10; 32], AccessRights::WRITE),
        URef::new([11; 32], AccessRights::WRITE),
        U512::from(15_000_000_000u64),
        U512::from(2_500_000_000u64),
        Some(1),
    );
    let deploy_info = DeployInfo::new(
        DeployHash::new([55; 32]),
        &[TransferAddr::new([1; 32]), TransferAddr::new([2; 32])],
        AccountHash::new([100; 32]),
        URef::new([10; 32], AccessRights::READ_ADD_WRITE),
        U512::from(2_500_000_000u64),
    );

    let validator_secret_key =
        SecretKey::ed25519_from_bytes([42; 32]).expect("should create secret key");
    let delegator_secret_key =
        SecretKey::secp256k1_from_bytes([43; 32]).expect("should create secret key");

    let era_info = {
        let mut era_info = EraInfo::new();

        era_info
            .seigniorage_allocations_mut()
            .push(SeigniorageAllocation::Validator {
                validator_public_key: PublicKey::from(&validator_secret_key),
                amount: U512::from(1_000_000_000),
            });

        era_info
            .seigniorage_allocations_mut()
            .push(SeigniorageAllocation::Delegator {
                validator_public_key: PublicKey::from(&validator_secret_key),
                delegator_public_key: PublicKey::from(&delegator_secret_key),
                amount: U512::from(1_000_000_000),
            });
        era_info
    };

    let bid = Bid::locked(
        PublicKey::from(&validator_secret_key),
        URef::new([10; 32], AccessRights::READ_ADD_WRITE),
        U512::from(50_000_000_000u64),
        100,
        u64::MAX,
    );
    let unbonding_purse_1 = UnbondingPurse::new(
        URef::new([10; 32], AccessRights::READ),
        PublicKey::from(&validator_secret_key),
        PublicKey::from(&validator_secret_key),
        EraId::new(41),
        U512::from(60_000_000_000u64),
    );
    let unbonding_purse_2 = UnbondingPurse::new(
        URef::new([11; 32], AccessRights::READ),
        PublicKey::from(&validator_secret_key),
        PublicKey::from(&delegator_secret_key),
        EraId::new(42),
        U512::from(50_000_000_000u64),
    );

    let transform = {
        let mut transform = BTreeMap::new();
        transform.insert(
            "Identity".to_string(),
            ABITestCase::from_inputs(vec![Transform::Identity.into()])?,
        );

        let write_string =
            CLValue::from_t("Hello, world!".to_string()).expect("should create clvalue of string");
        transform.insert(
            "WriteCLValue".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteCLValue(write_string).into()])?,
        );
        transform.insert(
            "WriteAccount".to_string(),
            ABITestCase::from_inputs(vec![
                Transform::WriteAccount(AccountHash::new([42; 32])).into()
            ])?,
        );
        transform.insert(
            "WriteContractWasm".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteContractWasm.into()])?,
        );
        transform.insert(
            "WriteContract".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteContract.into()])?,
        );
        transform.insert(
            "WriteContractPackage".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteContractPackage.into()])?,
        );

        transform.insert(
            "WriteDeployInfo".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteDeployInfo(deploy_info.clone()).into()])?,
        );

        transform.insert(
            "WriteEraInfo".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteEraInfo(era_info.clone()).into()])?,
        );

        transform.insert(
            "WriteTransfer".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteTransfer(transfer).into()])?,
        );

        transform.insert(
            "WriteBid".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteBid(Box::new(bid.clone())).into()])?,
        );

        transform.insert(
            "WriteWithdraw".to_string(),
            ABITestCase::from_inputs(vec![Transform::WriteWithdraw(vec![
                unbonding_purse_1.clone(),
                unbonding_purse_2.clone(),
            ])
            .into()])?,
        );
        transform.insert(
            "AddInt32".to_string(),
            ABITestCase::from_inputs(vec![Transform::AddInt32(i32::MAX).into()])?,
        );
        transform.insert(
            "AddUInt64".to_string(),
            ABITestCase::from_inputs(vec![Transform::AddUInt64(u64::MAX).into()])?,
        );
        transform.insert(
            "AddUInt128".to_string(),
            ABITestCase::from_inputs(vec![Transform::AddUInt128(U128::MAX).into()])?,
        );
        transform.insert(
            "AddUInt256".to_string(),
            ABITestCase::from_inputs(vec![Transform::AddUInt256(U256::MAX).into()])?,
        );
        transform.insert(
            "AddUInt512".to_string(),
            ABITestCase::from_inputs(vec![Transform::AddUInt512(U512::MAX).into()])?,
        );

        let named_keys = {
            let mut named_keys = Vec::new();
            let key_hash = Key::Hash([42; 32]);
            let key_uref = Key::URef(URef::new([43; 32], AccessRights::READ_ADD_WRITE));
            named_keys.push(NamedKey {
                name: "key 1".to_string(),
                key: key_hash.to_formatted_string(),
            });
            named_keys.push(NamedKey {
                name: "uref".to_string(),
                key: key_uref.to_formatted_string(),
            });
            named_keys
        };

        transform.insert(
            "AddKeys".to_string(),
            ABITestCase::from_inputs(vec![Transform::AddKeys(named_keys).into()])?,
        );
        transform.insert(
            "Failure".to_string(),
            ABITestCase::from_inputs(vec![Transform::Failure(
                "This is error message".to_string(),
            )
            .into()])?,
        );
        Fixture::ABI("transform".to_string(), ABIFixture::from(transform))
    };

    let stored_value = {
        let mut stored_value = BTreeMap::new();

        let cl_value = CLValue::from_t("Hello, world!").expect("should create cl value");

        stored_value.insert(
            "CLValue".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::CLValue(cl_value).into()])?,
        );

        let account_secret_key =
            SecretKey::ed25519_from_bytes([42; 32]).expect("should create secret key");
        let account_public_key = PublicKey::from(&account_secret_key);
        let account_hash = account_public_key.to_account_hash();

        let account_named_keys = {
            let mut named_keys = NamedKeys::new();
            named_keys.insert("hash".to_string(), Key::Hash([42; 32]));
            named_keys.insert(
                "uref".to_string(),
                Key::URef(URef::new([16; 32], AccessRights::READ_ADD_WRITE)),
            );
            named_keys
        };

        let associated_keys = AssociatedKeys::new(account_hash, Weight::new(1));

        let action_thresholds = Account::new(
            account_hash,
            account_named_keys,
            URef::new([17; 32], AccessRights::WRITE),
            associated_keys,
            ActionThresholds::new(Weight::new(1), Weight::new(1)).unwrap(),
        );

        let account = action_thresholds;
        stored_value.insert(
            "Account".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::Account(account).into()])?,
        );

        let contract_wasm = ContractWasm::new(wasm::do_nothing_bytes());

        stored_value.insert(
            "ContractWasm".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::ContractWasm(contract_wasm).into()])?,
        );

        let contract_named_keys = {
            let mut named_keys = NamedKeys::new();
            named_keys.insert("hash".to_string(), Key::Hash([43; 32]));
            named_keys.insert(
                "uref".to_string(),
                Key::URef(URef::new([17; 32], AccessRights::READ_ADD_WRITE)),
            );
            named_keys
        };

        let entry_points = {
            let mut entry_points = EntryPoints::new();
            let public_contract_entry_point = EntryPoint::new(
                "public_entry_point_func",
                vec![
                    Parameter::new("param1", U512::cl_type()),
                    Parameter::new("param2", String::cl_type()),
                ],
                CLType::Unit,
                EntryPointAccess::Public,
                EntryPointType::Contract,
            );

            entry_points.add_entry_point(public_contract_entry_point);

            entry_points
        };

        let contract = Contract::new(
            ContractPackageHash::new([100; 32]),
            ContractWasmHash::new([101; 32]),
            contract_named_keys,
            entry_points,
            ProtocolVersion::V1_0_0,
        );
        stored_value.insert(
            "Contract".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::Contract(contract).into()])?,
        );

        let mut active_versions = ContractVersions::new();
        let v1_hash = ContractHash::new([99; 32]);
        let v2_hash = ContractHash::new([100; 32]);
        active_versions.insert(ContractVersionKey::new(1, 2), v1_hash);
        let v1 = ContractVersionKey::new(1, 1);
        active_versions.insert(v1, v2_hash);

        let mut disabled_versions = DisabledVersions::new();
        disabled_versions.insert(v1);

        let mut groups = Groups::new();
        groups.insert(Group::new("Empty"), BTreeSet::new());
        groups.insert(
            Group::new("Single"),
            BTreeSet::from_iter(vec![URef::new([55; 32], AccessRights::READ)]),
        );

        let contract_package = ContractPackage::new(
            URef::new([39; 32], AccessRights::READ),
            active_versions,
            disabled_versions,
            groups,
            ContractPackageStatus::Locked,
        );

        stored_value.insert(
            "ContractPackage".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::ContractPackage(contract_package).into()])?,
        );

        stored_value.insert(
            "Transfer".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::Transfer(transfer).into()])?,
        );
        stored_value.insert(
            "DeployInfo".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::DeployInfo(deploy_info).into()])?,
        );
        stored_value.insert(
            "EraInfo".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::EraInfo(era_info).into()])?,
        );
        stored_value.insert(
            "Bid".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::Bid(Box::new(bid)).into()])?,
        );
        stored_value.insert(
            "Withdraw".to_string(),
            ABITestCase::from_inputs(vec![StoredValue::Withdraw(vec![
                unbonding_purse_1,
                unbonding_purse_2,
            ])
            .into()])?,
        );

        Fixture::ABI("stored_value".to_string(), ABIFixture::from(stored_value))
    };

    Ok(vec![basic, transform, stored_value])
}
