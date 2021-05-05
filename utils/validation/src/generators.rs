use std::{
    collections::{BTreeMap, BTreeSet},
    iter::FromIterator,
};

use num_traits::Zero;

use casper_execution_engine::{
    core::engine_state::ExecutableDeployItem,
    shared::{
        account::{Account, ActionThresholds, AssociatedKeys},
        stored_value::StoredValue,
        wasm,
    },
};
use casper_types::{
    account::{AccountHash, Weight},
    bytesrepr::Bytes,
    contracts::{ContractPackageStatus, ContractVersions, DisabledVersions, Groups, NamedKeys},
    runtime_args,
    system::{
        auction::{Bid, EraInfo, SeigniorageAllocation, UnbondingPurse},
        mint,
    },
    AccessRights, CLType, CLTyped, CLValue, Contract, ContractHash, ContractPackage,
    ContractPackageHash, ContractVersionKey, ContractWasm, ContractWasmHash, DeployHash,
    DeployInfo, EntryPoint, EntryPointAccess, EntryPointType, EntryPoints, EraId, ExecutionEffect,
    ExecutionResult, Group, Key, NamedKey, OpKind, Operation, Parameter, ProtocolVersion,
    PublicKey, RuntimeArgs, SecretKey, Transfer, TransferAddr, Transform, TransformEntry, URef,
    U128, U256, U512,
};
use casper_validation::{
    abi::{ABIFixture, ABITestCase},
    error::Error,
    Fixture, TestFixtures,
};

fn make_keys() -> Vec<(String, Key)> {
    vec![
        (
            "Account".to_string(),
            Key::Account(AccountHash::new([42; 32])),
        ),
        ("Hash".to_string(), Key::Hash([43; 32])),
        (
            "URef".to_string(),
            Key::URef(URef::new([44; 32], AccessRights::NONE)),
        ),
        (
            "URef_READ".to_string(),
            Key::URef(URef::new([45; 32], AccessRights::READ)),
        ),
        (
            "URef_ADD".to_string(),
            Key::URef(URef::new([46; 32], AccessRights::ADD)),
        ),
        (
            "URef_WRITE".to_string(),
            Key::URef(URef::new([47; 32], AccessRights::WRITE)),
        ),
        (
            "Transfer".to_string(),
            Key::Transfer(TransferAddr::new([48; 32])),
        ),
        (
            "DeployInfo".to_string(),
            Key::DeployInfo(DeployHash::new([49; 32])),
        ),
        ("EraInfo".to_string(), Key::EraInfo(EraId::from(50))),
        ("Balance".to_string(), Key::Balance([51; 32])),
        ("Bid".to_string(), Key::Bid(AccountHash::new([52; 32]))),
        (
            "Withdraw".to_string(),
            Key::Withdraw(AccountHash::new([53; 32])),
        ),
    ]
}

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
            named_keys.push(NamedKey::new("key 1".to_string(), key_hash));
            named_keys.push(NamedKey::new("uref".to_string(), key_uref));
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
                "public_contract_entry_point_func",
                vec![
                    Parameter::new("param1", U512::cl_type()),
                    Parameter::new("param2", String::cl_type()),
                ],
                CLType::Unit,
                EntryPointAccess::Public,
                EntryPointType::Contract,
            );

            entry_points.add_entry_point(public_contract_entry_point);

            let public_session_entry_point = EntryPoint::new(
                "public_session_entry_point_func",
                vec![
                    Parameter::new("param1", U512::cl_type()),
                    Parameter::new("param2", String::cl_type()),
                ],
                CLType::Unit,
                EntryPointAccess::Public,
                EntryPointType::Session,
            );

            entry_points.add_entry_point(public_session_entry_point);

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

    let clvalue = {
        let mut clvalue = BTreeMap::new();

        // CL_TYPE_TAG_BOOL
        let bool_value_true = CLValue::from_t(true).unwrap();
        clvalue.insert(
            "bool_value_true".to_string(),
            ABITestCase::from_inputs(vec![bool_value_true.into()]).unwrap(),
        );

        let bool_value_false = CLValue::from_t(false).unwrap();
        clvalue.insert(
            "bool_value_false".to_string(),
            ABITestCase::from_inputs(vec![bool_value_false.into()]).unwrap(),
        );

        // CL_TYPE_TAG_I32
        let i32_value_positive = CLValue::from_t(i32::MAX).unwrap();
        clvalue.insert(
            "i32_value_positive".to_string(),
            ABITestCase::from_inputs(vec![i32_value_positive.into()]).unwrap(),
        );

        let i32_value_negative = CLValue::from_t(i32::MIN).unwrap();
        clvalue.insert(
            "i32_value_negative".to_string(),
            ABITestCase::from_inputs(vec![i32_value_negative.into()]).unwrap(),
        );

        // CL_TYPE_TAG_I64
        let i64_value_positive = CLValue::from_t(i64::MAX).unwrap();
        clvalue.insert(
            "i64_value_positive".to_string(),
            ABITestCase::from_inputs(vec![i64_value_positive.into()]).unwrap(),
        );

        let i64_value_negative = CLValue::from_t(i64::MIN).unwrap();
        clvalue.insert(
            "i64_value_negative".to_string(),
            ABITestCase::from_inputs(vec![i64_value_negative.into()]).unwrap(),
        );

        // CL_TYPE_TAG_U8
        let u8_value_positive = CLValue::from_t(u8::MAX).unwrap();
        clvalue.insert(
            "u8_value_positive".to_string(),
            ABITestCase::from_inputs(vec![u8_value_positive.into()]).unwrap(),
        );

        let u8_value_zero = CLValue::from_t(u8::zero()).unwrap();
        clvalue.insert(
            "u8_value_zero".to_string(),
            ABITestCase::from_inputs(vec![u8_value_zero.into()]).unwrap(),
        );

        // CL_TYPE_TAG_U32
        let u32_value_positive = CLValue::from_t(u32::MAX).unwrap();
        clvalue.insert(
            "u32_value_positive".to_string(),
            ABITestCase::from_inputs(vec![u32_value_positive.into()]).unwrap(),
        );

        let u32_value_zero = CLValue::from_t(u32::zero()).unwrap();
        clvalue.insert(
            "u32_value_zero".to_string(),
            ABITestCase::from_inputs(vec![u32_value_zero.into()]).unwrap(),
        );

        // CL_TYPE_TAG_U64
        let u64_value_positive = CLValue::from_t(u64::MAX).unwrap();
        clvalue.insert(
            "u64_value_positive".to_string(),
            ABITestCase::from_inputs(vec![u64_value_positive.into()]).unwrap(),
        );

        let u64_value_zero = CLValue::from_t(u64::zero()).unwrap();
        clvalue.insert(
            "u64_value_zero".to_string(),
            ABITestCase::from_inputs(vec![u64_value_zero.into()]).unwrap(),
        );

        // CL_TYPE_TAG_U128
        let u64_value_positive = CLValue::from_t(U128::MAX).unwrap();
        clvalue.insert(
            "u64_value_positive".to_string(),
            ABITestCase::from_inputs(vec![u64_value_positive.into()]).unwrap(),
        );

        let u64_value_zero = CLValue::from_t(U128::zero()).unwrap();
        clvalue.insert(
            "u64_value_zero".to_string(),
            ABITestCase::from_inputs(vec![u64_value_zero.into()]).unwrap(),
        );

        // CL_TYPE_TAG_U256
        let u256_value_positive = CLValue::from_t(U256::MAX).unwrap();
        clvalue.insert(
            "u256_value_positive".to_string(),
            ABITestCase::from_inputs(vec![u256_value_positive.into()]).unwrap(),
        );

        let u256_value_zero = CLValue::from_t(U256::zero()).unwrap();
        clvalue.insert(
            "u256_value_zero".to_string(),
            ABITestCase::from_inputs(vec![u256_value_zero.into()]).unwrap(),
        );

        // CL_TYPE_TAG_U512
        let u512_value_positive = CLValue::from_t(U512::MAX).unwrap();
        clvalue.insert(
            "u512_value_positive".to_string(),
            ABITestCase::from_inputs(vec![u512_value_positive.into()]).unwrap(),
        );

        let u512_value_zero = CLValue::from_t(U512::zero()).unwrap();
        clvalue.insert(
            "u512_value_zero".to_string(),
            ABITestCase::from_inputs(vec![u512_value_zero.into()]).unwrap(),
        );

        // CL_TYPE_TAG_UNIT
        let unit_value = CLValue::unit();
        clvalue.insert(
            "unit".to_string(),
            ABITestCase::from_inputs(vec![unit_value.into()]).unwrap(),
        );

        // CL_TYPE_TAG_STRING
        let string_value = CLValue::from_t("Hello, world!".to_string()).unwrap();
        clvalue.insert(
            "string_value".to_string(),
            ABITestCase::from_inputs(vec![string_value.into()]).unwrap(),
        );

        // CL_TYPE_TAG_KEY
        for (description, key) in make_keys() {
            let clvalue_key = CLValue::from_t(key).unwrap();
            clvalue.insert(
                description,
                ABITestCase::from_inputs(vec![clvalue_key.into()]).unwrap(),
            );
        }

        // CL_TYPE_TAG_UREF
        let clvalue_uref =
            CLValue::from_t(URef::new([254; 32], AccessRights::READ_ADD_WRITE)).unwrap();
        clvalue.insert(
            "clvalue_uref".to_string(),
            ABITestCase::from_inputs(vec![clvalue_uref.into()]).unwrap(),
        );
        // CL_TYPE_TAG_OPTION
        let clvalue_optional_string_none = CLValue::from_t(Option::<String>::None).unwrap();
        clvalue.insert(
            "clvalue_optional_string_none".to_string(),
            ABITestCase::from_inputs(vec![clvalue_optional_string_none.into()]).unwrap(),
        );
        let clvalue_optional_string_some =
            CLValue::from_t(Some("Hello, world!".to_string())).unwrap();
        clvalue.insert(
            "clvalue_optional_string_some".to_string(),
            ABITestCase::from_inputs(vec![clvalue_optional_string_some.into()]).unwrap(),
        );

        // CL_TYPE_TAG_LIST
        let clvalue_vec_of_string = CLValue::from_t(vec![
            "Hello".to_string(),
            "world".to_string(),
            "!".to_string(),
        ])
        .unwrap();
        clvalue.insert(
            "clvalue_vec_of_string".to_string(),
            ABITestCase::from_inputs(vec![clvalue_vec_of_string.into()]).unwrap(),
        );

        let clvalue_vec_of_u512 = CLValue::from_t(vec![
            U512::zero(),
            U512::from(u8::MAX),
            U512::from(u16::MAX),
            U512::from(u32::MAX),
            U512::from(u64::MAX),
            U512::MAX,
        ])
        .unwrap();
        clvalue.insert(
            "clvalue_vec_of_u512".to_string(),
            ABITestCase::from_inputs(vec![clvalue_vec_of_u512.into()]).unwrap(),
        );
        // CL_TYPE_TAG_BYTE_ARRAY
        let clvalue_bytearray_8b = CLValue::from_t([255; 8]).unwrap();
        clvalue.insert(
            "clvalue_bytearray_8b".to_string(),
            ABITestCase::from_inputs(vec![clvalue_bytearray_8b.into()]).unwrap(),
        );
        let clvalue_bytearray_32b = CLValue::from_t([42u8; 32]).unwrap();
        clvalue.insert(
            "clvalue_bytearray_32b".to_string(),
            ABITestCase::from_inputs(vec![clvalue_bytearray_32b.into()]).unwrap(),
        );
        let clvalue_bytearray_256b = CLValue::from_t([254; 256]).unwrap();
        clvalue.insert(
            "clvalue_bytearray_256b".to_string(),
            ABITestCase::from_inputs(vec![clvalue_bytearray_256b.into()]).unwrap(),
        );
        // CL_TYPE_TAG_RESULT
        type TestResult = Result<String, u32>;
        let clvalue_ok = CLValue::from_t(TestResult::Ok("Success".to_string())).unwrap();
        clvalue.insert(
            "clvalue_ok".to_string(),
            ABITestCase::from_inputs(vec![clvalue_ok.into()]).unwrap(),
        );
        let clvalue_err = CLValue::from_t(TestResult::Err(u32::MAX)).unwrap();
        clvalue.insert(
            "clvalue_err".to_string(),
            ABITestCase::from_inputs(vec![clvalue_err.into()]).unwrap(),
        );
        // CL_TYPE_TAG_MAP

        let clvalue_string_to_u512 = {
            let mut test_map = BTreeMap::new();
            test_map.insert("Alice".to_string(), U512::MAX);
            test_map.insert("Bob".to_string(), U512::from(u64::MAX) + U512::one());
            CLValue::from_t(test_map).unwrap()
        };
        clvalue.insert(
            "clvalue_map_string_to_u512".to_string(),
            ABITestCase::from_inputs(vec![clvalue_string_to_u512.into()]).unwrap(),
        );
        // CL_TYPE_TAG_TUPLE1
        let clvalue_tuple_1 = CLValue::from_t(("Hello, world!".to_string(),)).unwrap();
        clvalue.insert(
            "clvalue_tuple_1".to_string(),
            ABITestCase::from_inputs(vec![clvalue_tuple_1.into()]).unwrap(),
        );
        // CL_TYPE_TAG_TUPLE2
        let clvalue_tuple_2 = CLValue::from_t((U512::zero(), U512::MAX)).unwrap();
        clvalue.insert(
            "clvalue_tuple_2".to_string(),
            ABITestCase::from_inputs(vec![clvalue_tuple_2.into()]).unwrap(),
        );
        // CL_TYPE_TAG_TUPLE3
        let clvalue_tuple_3 =
            CLValue::from_t((U512::zero(), U512::from(u64::MAX) + U512::one(), U512::MAX)).unwrap();
        clvalue.insert(
            "clvalue_tuple_3".to_string(),
            ABITestCase::from_inputs(vec![clvalue_tuple_3.into()]).unwrap(),
        );
        // CL_TYPE_TAG_ANY
        let clvalue_any =
            CLValue::from_components(CLType::Any, 0xdeadbeefu32.to_be_bytes().to_vec());
        clvalue.insert(
            "clvalue_any".to_string(),
            ABITestCase::from_inputs(vec![clvalue_any.into()]).unwrap(),
        );
        // CL_TYPE_TAG_PUBLIC_KEY
        let ed25519_secret_key = SecretKey::ed25519_from_bytes(&[42; 32]).unwrap();
        let ed25519_pubkey = PublicKey::from(&ed25519_secret_key);
        let clvalue_ed25519_pubkey = CLValue::from_t(ed25519_pubkey).unwrap();
        clvalue.insert(
            "clvalue_ed25519_pubkey".to_string(),
            ABITestCase::from_inputs(vec![clvalue_ed25519_pubkey.into()]).unwrap(),
        );

        let ed25519_secret_key = SecretKey::ed25519_from_bytes(&[42; 32]).unwrap();
        let ed25519_pubkey = PublicKey::from(&ed25519_secret_key);
        let clvalue_ed25519_pubkey = CLValue::from_t(ed25519_pubkey).unwrap();
        clvalue.insert(
            "clvalue_ed25519_pubkey".to_string(),
            ABITestCase::from_inputs(vec![clvalue_ed25519_pubkey.into()]).unwrap(),
        );

        let secp256k1_secret_key = SecretKey::secp256k1_from_bytes(&[42; 32]).unwrap();
        let secp256k1_pubkey = PublicKey::from(&secp256k1_secret_key);
        let clvalue_secp256k1_pubkey = CLValue::from_t(secp256k1_pubkey).unwrap();
        clvalue.insert(
            "clvalue_secp256k1_pubkey".to_string(),
            ABITestCase::from_inputs(vec![clvalue_secp256k1_pubkey.into()]).unwrap(),
        );

        let clvalue_string_to_list_of_optional_u512 = {
            let mut test_map = BTreeMap::new();
            test_map.insert(
                "Alice".to_string(),
                vec![Some(U512::zero()), Some(U512::MAX), None],
            );
            test_map.insert(
                "Bob".to_string(),
                vec![Some(U512::from(u64::MAX) + U512::one()), None],
            );
            test_map.insert("Charlie".to_string(), vec![None, None, None]);
            CLValue::from_t(test_map).unwrap()
        };
        clvalue.insert(
            "clvalue_string_to_list_of_optional_u512".to_string(),
            ABITestCase::from_inputs(vec![clvalue_string_to_list_of_optional_u512.into()]).unwrap(),
        );

        Fixture::ABI("clvalue".to_string(), ABIFixture::from(clvalue))
    };

    let execution_result = {
        let mut execution_result = BTreeMap::new();

        let key_hash = Key::Hash([42; 32]);
        let key_balance = Key::Balance([43; 32]);
        let key_uref = Key::URef(URef::new([44; 32], AccessRights::READ_ADD_WRITE));
        let key_account = Key::Account(AccountHash::new([123; 32]));

        let effect = ExecutionEffect {
            operations: vec![
                Operation::new(key_hash, OpKind::Read),
                Operation::new(key_balance, OpKind::Write),
                Operation::new(key_uref, OpKind::Add),
                Operation::new(key_account, OpKind::NoOp),
            ],
            transforms: vec![
                TransformEntry::new(key_hash, Transform::WriteContractWasm),
                TransformEntry::new(key_balance, Transform::AddUInt512(U512::one())),
            ],
        };

        let success = ExecutionResult::Success {
            effect,
            transfers: vec![TransferAddr::new([100; 32]), TransferAddr::new([101; 32])],
            cost: U512::from(2_500_020_000u64),
        };

        let failure = ExecutionResult::Failure {
            effect: ExecutionEffect::default(),
            transfers: Vec::new(),
            cost: U512::from(2_500_000_000u64),
            error_message: "Error message".to_string(),
        };

        execution_result.insert(
            "Success".to_string(),
            ABITestCase::from_inputs(vec![success.into()]).unwrap(),
        );
        execution_result.insert(
            "Failure_NoEffects_NoTransfers".to_string(),
            ABITestCase::from_inputs(vec![failure.into()]).unwrap(),
        );

        Fixture::ABI(
            "execution_result".to_string(),
            ABIFixture::from(execution_result),
        )
    };

    let non_empty_args = runtime_args! {
        "name" => String::from("John"),
    };

    let contract_hash = ContractHash::new([42; 32]);
    let contract_package_hash = ContractPackageHash::new([42; 32]);
    const ENTRY_POINT: &str = "entry_point";
    const CONTRACT_NAME: &str = "contract";
    const CONTRACT_PACKAGE_NAME: &str = "contract_package";

    const ACCOUNT_ADDR: AccountHash = AccountHash::new([99; 32]);

    let transfer_args = runtime_args! {
        mint::ARG_TARGET => ACCOUNT_ADDR,
        mint::ARG_AMOUNT => U512::from(2_500_000_000u64),
        mint::ARG_ID => Some(123),
    };

    let executable_deploy_item = {
        let mut executable_deploy_item = BTreeMap::new();
        let item = ExecutableDeployItem::ModuleBytes {
            module_bytes: Bytes::from(wasm::do_nothing_bytes()),
            args: non_empty_args.clone(),
        };
        executable_deploy_item.insert(
            "ModuleBytes".to_string(),
            ABITestCase::from_inputs(vec![item.into()]).unwrap(),
        );
        let item = ExecutableDeployItem::StoredContractByHash {
            hash: contract_hash,
            entry_point: ENTRY_POINT.to_string(),
            args: non_empty_args.clone(),
        };
        executable_deploy_item.insert(
            "StoredContractByHash".to_string(),
            ABITestCase::from_inputs(vec![item.into()]).unwrap(),
        );
        let item = ExecutableDeployItem::StoredContractByName {
            name: CONTRACT_NAME.to_string(),
            entry_point: ENTRY_POINT.to_string(),
            args: non_empty_args.clone(),
        };
        executable_deploy_item.insert(
            "StoredContractByName".to_string(),
            ABITestCase::from_inputs(vec![item.into()]).unwrap(),
        );
        let item = ExecutableDeployItem::StoredVersionedContractByHash {
            hash: contract_package_hash,
            version: Some(123),
            entry_point: ENTRY_POINT.to_string(),
            args: non_empty_args.clone(),
        };
        executable_deploy_item.insert(
            "StoredVersionedContractByHash".to_string(),
            ABITestCase::from_inputs(vec![item.into()]).unwrap(),
        );
        let item = ExecutableDeployItem::StoredVersionedContractByName {
            name: CONTRACT_PACKAGE_NAME.to_string(),
            version: None,
            entry_point: ENTRY_POINT.to_string(),
            args: non_empty_args,
        };
        executable_deploy_item.insert(
            "StoredVersionedContractByName".to_string(),
            ABITestCase::from_inputs(vec![item.into()]).unwrap(),
        );
        let item = ExecutableDeployItem::Transfer {
            args: transfer_args,
        };
        executable_deploy_item.insert(
            "Transfer".to_string(),
            ABITestCase::from_inputs(vec![item.into()]).unwrap(),
        );

        Fixture::ABI(
            "executable_deploy_item".to_string(),
            ABIFixture::from(executable_deploy_item),
        )
    };

    Ok(vec![
        basic,
        transform,
        stored_value,
        clvalue,
        execution_result,
        executable_deploy_item,
    ])
}
