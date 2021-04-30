//! Contains functions for generating arbitrary values for use by
//! [`Proptest`](https://crates.io/crates/proptest).
#![allow(missing_docs)]

use alloc::{boxed::Box, string::String, vec};

use proptest::{
    array, bits,
    collection::{btree_map, btree_set, vec},
    option,
    prelude::*,
    result,
};

use crate::{
    account::{AccountHash, Weight},
    contracts::{
        ContractPackageStatus, ContractVersions, DisabledVersions, Groups, NamedKeys, Parameters,
    },
    transfer::TransferAddr,
    AccessRights, CLType, CLValue, Contract, ContractHash, ContractPackage, ContractVersionKey,
    ContractWasm, EntryPoint, EntryPointAccess, EntryPointType, EntryPoints, EraId, Group, Key,
    NamedArg, NamedKey, Parameter, Phase, ProtocolVersion, SemVer, URef, U128, U256, U512,
};

use crate::deploy_info::gens::{deploy_hash_arb, transfer_addr_arb};
pub use crate::{deploy_info::gens::deploy_info_arb, transfer::gens::transfer_arb};

pub fn u8_slice_32() -> impl Strategy<Value = [u8; 32]> {
    vec(any::<u8>(), 32).prop_map(|b| {
        let mut res = [0u8; 32];
        res.clone_from_slice(b.as_slice());
        res
    })
}

pub fn u2_slice_32() -> impl Strategy<Value = [u8; 32]> {
    array::uniform32(any::<u8>()).prop_map(|mut arr| {
        for byte in arr.iter_mut() {
            *byte &= 0b11;
        }
        arr
    })
}

pub fn named_keys_arb(depth: usize) -> impl Strategy<Value = NamedKeys> {
    btree_map("\\PC*", key_arb(), depth)
}

pub fn named_key_arb() -> impl Strategy<Value = NamedKey> {
    ("\\PC*", key_arb()).prop_map(|(key, value)| NamedKey::new(key, value))
}

pub fn access_rights_arb() -> impl Strategy<Value = AccessRights> {
    prop_oneof![
        Just(AccessRights::NONE),
        Just(AccessRights::READ),
        Just(AccessRights::ADD),
        Just(AccessRights::WRITE),
        Just(AccessRights::READ_ADD),
        Just(AccessRights::READ_WRITE),
        Just(AccessRights::ADD_WRITE),
        Just(AccessRights::READ_ADD_WRITE),
    ]
}

pub fn phase_arb() -> impl Strategy<Value = Phase> {
    prop_oneof![
        Just(Phase::Payment),
        Just(Phase::Session),
        Just(Phase::FinalizePayment),
    ]
}

pub fn uref_arb() -> impl Strategy<Value = URef> {
    (array::uniform32(bits::u8::ANY), access_rights_arb())
        .prop_map(|(id, access_rights)| URef::new(id, access_rights))
}

pub fn era_id_arb() -> impl Strategy<Value = EraId> {
    any::<u64>().prop_map(EraId::from)
}

pub fn key_arb() -> impl Strategy<Value = Key> {
    prop_oneof![
        account_hash_arb().prop_map(Key::Account),
        u8_slice_32().prop_map(Key::Hash),
        uref_arb().prop_map(Key::URef),
        transfer_addr_arb().prop_map(Key::Transfer),
        deploy_hash_arb().prop_map(Key::DeployInfo),
        era_id_arb().prop_map(Key::EraInfo),
        uref_arb().prop_map(|uref| Key::Balance(uref.addr())),
        account_hash_arb().prop_map(Key::Bid),
        account_hash_arb().prop_map(Key::Withdraw),
    ]
}

pub fn colliding_key_arb() -> impl Strategy<Value = Key> {
    prop_oneof![
        u2_slice_32().prop_map(|bytes| Key::Account(AccountHash::new(bytes))),
        u2_slice_32().prop_map(Key::Hash),
        u2_slice_32().prop_map(|bytes| Key::URef(URef::new(bytes, AccessRights::NONE))),
        u2_slice_32().prop_map(|bytes| Key::Transfer(TransferAddr::new(bytes))),
    ]
}

pub fn account_hash_arb() -> impl Strategy<Value = AccountHash> {
    u8_slice_32().prop_map(AccountHash::new)
}

pub fn weight_arb() -> impl Strategy<Value = Weight> {
    any::<u8>().prop_map(Weight::new)
}

pub fn sem_ver_arb() -> impl Strategy<Value = SemVer> {
    (any::<u32>(), any::<u32>(), any::<u32>())
        .prop_map(|(major, minor, patch)| SemVer::new(major, minor, patch))
}

pub fn protocol_version_arb() -> impl Strategy<Value = ProtocolVersion> {
    sem_ver_arb().prop_map(ProtocolVersion::new)
}

pub fn u128_arb() -> impl Strategy<Value = U128> {
    vec(any::<u8>(), 0..16).prop_map(|b| U128::from_little_endian(b.as_slice()))
}

pub fn u256_arb() -> impl Strategy<Value = U256> {
    vec(any::<u8>(), 0..32).prop_map(|b| U256::from_little_endian(b.as_slice()))
}

pub fn u512_arb() -> impl Strategy<Value = U512> {
    vec(any::<u8>(), 0..64).prop_map(|b| U512::from_little_endian(b.as_slice()))
}

pub fn cl_simple_type_arb() -> impl Strategy<Value = CLType> {
    prop_oneof![
        Just(CLType::Bool),
        Just(CLType::I32),
        Just(CLType::I64),
        Just(CLType::U8),
        Just(CLType::U32),
        Just(CLType::U64),
        Just(CLType::U128),
        Just(CLType::U256),
        Just(CLType::U512),
        Just(CLType::Unit),
        Just(CLType::String),
        Just(CLType::Key),
        Just(CLType::URef),
    ]
}

pub fn cl_type_arb() -> impl Strategy<Value = CLType> {
    cl_simple_type_arb().prop_recursive(4, 16, 8, |element| {
        prop_oneof![
            // We want to produce basic types too
            element.clone(),
            // For complex type
            element
                .clone()
                .prop_map(|val| CLType::Option(Box::new(val))),
            element.clone().prop_map(|val| CLType::List(Box::new(val))),
            // Realistic Result type generator: ok is anything recursive, err is simple type
            (element.clone(), cl_simple_type_arb()).prop_map(|(ok, err)| CLType::Result {
                ok: Box::new(ok),
                err: Box::new(err)
            }),
            // Realistic Map type generator: key is simple type, value is complex recursive type
            (cl_simple_type_arb(), element.clone()).prop_map(|(key, value)| CLType::Map {
                key: Box::new(key),
                value: Box::new(value)
            }),
            // Various tuples
            element
                .clone()
                .prop_map(|cl_type| CLType::Tuple1([Box::new(cl_type)])),
            (element.clone(), element.clone()).prop_map(|(cl_type1, cl_type2)| CLType::Tuple2([
                Box::new(cl_type1),
                Box::new(cl_type2)
            ])),
            (element.clone(), element.clone(), element).prop_map(
                |(cl_type1, cl_type2, cl_type3)| CLType::Tuple3([
                    Box::new(cl_type1),
                    Box::new(cl_type2),
                    Box::new(cl_type3)
                ])
            ),
        ]
    })
}

pub fn cl_value_arb() -> impl Strategy<Value = CLValue> {
    // If compiler brings you here it most probably means you've added a variant to `CLType` enum
    // but forgot to add generator for it.
    let stub: Option<CLType> = None;
    if let Some(cl_type) = stub {
        match cl_type {
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
            | CLType::Key
            | CLType::URef
            | CLType::PublicKey
            | CLType::Option(_)
            | CLType::List(_)
            | CLType::ByteArray(..)
            | CLType::Result { .. }
            | CLType::Map { .. }
            | CLType::Tuple1(_)
            | CLType::Tuple2(_)
            | CLType::Tuple3(_)
            | CLType::Any => (),
        }
    };

    prop_oneof![
        Just(CLValue::from_t(()).expect("should create CLValue")),
        any::<bool>().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        any::<i32>().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        any::<i64>().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        any::<u8>().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        any::<u32>().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        any::<u64>().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        u128_arb().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        u256_arb().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        u512_arb().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        key_arb().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        uref_arb().prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        ".*".prop_map(|x: String| CLValue::from_t(x).expect("should create CLValue")),
        option::of(any::<u64>()).prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        vec(uref_arb(), 0..100).prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        result::maybe_err(key_arb(), ".*")
            .prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        btree_map(".*", u512_arb(), 0..100)
            .prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        (any::<bool>()).prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        (any::<bool>(), any::<i32>())
            .prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        (any::<bool>(), any::<i32>(), any::<i64>())
            .prop_map(|x| CLValue::from_t(x).expect("should create CLValue")),
        // Fixed lists of any size
        any::<u8>().prop_map(|len| CLValue::from_t([len; 32]).expect("should create CLValue")),
    ]
}

pub fn result_arb() -> impl Strategy<Value = Result<u32, u32>> {
    result::maybe_ok(any::<u32>(), any::<u32>())
}

pub fn named_args_arb() -> impl Strategy<Value = NamedArg> {
    (".*", cl_value_arb()).prop_map(|(name, value)| NamedArg::new(name, value))
}

pub fn group_arb() -> impl Strategy<Value = Group> {
    ".*".prop_map(Group::new)
}

pub fn entry_point_access_arb() -> impl Strategy<Value = EntryPointAccess> {
    prop_oneof![
        Just(EntryPointAccess::Public),
        vec(group_arb(), 0..32).prop_map(EntryPointAccess::Groups),
    ]
}

pub fn entry_point_type_arb() -> impl Strategy<Value = EntryPointType> {
    prop_oneof![
        Just(EntryPointType::Session),
        Just(EntryPointType::Contract),
    ]
}

pub fn parameter_arb() -> impl Strategy<Value = Parameter> {
    (".*", cl_type_arb()).prop_map(|(name, cl_type)| Parameter::new(name, cl_type))
}

pub fn parameters_arb() -> impl Strategy<Value = Parameters> {
    vec(parameter_arb(), 0..10)
}

pub fn entry_point_arb() -> impl Strategy<Value = EntryPoint> {
    (
        ".*",
        parameters_arb(),
        entry_point_type_arb(),
        entry_point_access_arb(),
        cl_type_arb(),
    )
        .prop_map(
            |(name, parameters, entry_point_type, entry_point_access, ret)| {
                EntryPoint::new(name, parameters, ret, entry_point_access, entry_point_type)
            },
        )
}

pub fn entry_points_arb() -> impl Strategy<Value = EntryPoints> {
    vec(entry_point_arb(), 1..10).prop_map(EntryPoints::from)
}

pub fn contract_arb() -> impl Strategy<Value = Contract> {
    (
        protocol_version_arb(),
        entry_points_arb(),
        u8_slice_32(),
        u8_slice_32(),
        named_keys_arb(20),
    )
        .prop_map(
            |(
                protocol_version,
                entry_points,
                contract_package_hash_arb,
                contract_wasm_hash,
                named_keys,
            )| {
                Contract::new(
                    contract_package_hash_arb.into(),
                    contract_wasm_hash.into(),
                    named_keys,
                    entry_points,
                    protocol_version,
                )
            },
        )
}

pub fn contract_wasm_arb() -> impl Strategy<Value = ContractWasm> {
    vec(any::<u8>(), 1..1000).prop_map(ContractWasm::new)
}

pub fn contract_version_key_arb() -> impl Strategy<Value = ContractVersionKey> {
    (1..32u32, 1..1000u32)
        .prop_map(|(major, contract_ver)| ContractVersionKey::new(major, contract_ver))
}

pub fn contract_versions_arb() -> impl Strategy<Value = ContractVersions> {
    btree_map(
        contract_version_key_arb(),
        u8_slice_32().prop_map(ContractHash::new),
        1..5,
    )
}

pub fn disabled_versions_arb() -> impl Strategy<Value = DisabledVersions> {
    btree_set(contract_version_key_arb(), 0..5)
}

pub fn groups_arb() -> impl Strategy<Value = Groups> {
    btree_map(group_arb(), btree_set(uref_arb(), 1..10), 0..5)
}

pub fn contract_package_arb() -> impl Strategy<Value = ContractPackage> {
    (
        uref_arb(),
        contract_versions_arb(),
        disabled_versions_arb(),
        groups_arb(),
    )
        .prop_map(|(access_key, versions, disabled_versions, groups)| {
            ContractPackage::new(
                access_key,
                versions,
                disabled_versions,
                groups,
                ContractPackageStatus::default(),
            )
        })
}
