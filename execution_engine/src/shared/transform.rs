use std::{
    any,
    convert::TryFrom,
    default::Default,
    fmt::{self, Display, Formatter},
    ops::{Add, AddAssign},
};

use num::traits::{AsPrimitive, WrappingAdd};

use casper_types::{
    bytesrepr::{self, FromBytes, ToBytes},
    contracts::NamedKeys,
    CLType, CLTyped, CLValue, CLValueError, U128, U256, U512,
};

use crate::shared::{stored_value::StoredValue, TypeMismatch};

/// Error type for applying and combining transforms. A `TypeMismatch`
/// occurs when a transform cannot be applied because the types are
/// not compatible (e.g. trying to add a number to a string). An
/// `Overflow` occurs if addition between numbers would result in the
/// value overflowing its size in memory (e.g. if a, b are i32 and a +
/// b > i32::MAX then a `AddInt32(a).apply(Value::Int32(b))` would
/// cause an overflow).
#[derive(PartialEq, Eq, Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Serialization(bytesrepr::Error),
    #[error(transparent)]
    TypeMismatch(#[from] TypeMismatch),
}

impl From<CLValueError> for Error {
    fn from(cl_value_error: CLValueError) -> Error {
        match cl_value_error {
            CLValueError::Serialization(error) => Error::Serialization(error),
            CLValueError::Type(cl_type_mismatch) => {
                let expected = format!("{:?}", cl_type_mismatch.expected);
                let found = format!("{:?}", cl_type_mismatch.found);
                let type_mismatch = TypeMismatch { expected, found };
                Error::TypeMismatch(type_mismatch)
            }
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Transform {
    Identity,
    Write(StoredValue),
    AddInt32(i32),
    AddUInt64(u64),
    AddUInt128(U128),
    AddUInt256(U256),
    AddUInt512(U512),
    AddKeys(NamedKeys),
    Failure(Error),
}

macro_rules! from_try_from_impl {
    ($type:ty, $variant:ident) => {
        impl From<$type> for Transform {
            fn from(x: $type) -> Self {
                Transform::$variant(x)
            }
        }

        impl TryFrom<Transform> for $type {
            type Error = String;

            fn try_from(t: Transform) -> Result<$type, String> {
                match t {
                    Transform::$variant(x) => Ok(x),
                    other => Err(format!("{:?}", other)),
                }
            }
        }
    };
}

from_try_from_impl!(StoredValue, Write);
from_try_from_impl!(i32, AddInt32);
from_try_from_impl!(u64, AddUInt64);
from_try_from_impl!(U128, AddUInt128);
from_try_from_impl!(U256, AddUInt256);
from_try_from_impl!(U512, AddUInt512);
from_try_from_impl!(NamedKeys, AddKeys);
from_try_from_impl!(Error, Failure);

/// Attempts a wrapping addition of `to_add` to `stored_value`, assuming `stored_value` is
/// compatible with type `Y`.
fn wrapping_addition<Y>(stored_value: StoredValue, to_add: Y) -> Result<StoredValue, Error>
where
    Y: AsPrimitive<i32>
        + AsPrimitive<i64>
        + AsPrimitive<u8>
        + AsPrimitive<u32>
        + AsPrimitive<u64>
        + AsPrimitive<U128>
        + AsPrimitive<U256>
        + AsPrimitive<U512>,
{
    let cl_value = CLValue::try_from(stored_value)?;

    match cl_value.cl_type() {
        CLType::I32 => do_wrapping_addition::<i32, _>(cl_value, to_add),
        CLType::I64 => do_wrapping_addition::<i64, _>(cl_value, to_add),
        CLType::U8 => do_wrapping_addition::<u8, _>(cl_value, to_add),
        CLType::U32 => do_wrapping_addition::<u32, _>(cl_value, to_add),
        CLType::U64 => do_wrapping_addition::<u64, _>(cl_value, to_add),
        CLType::U128 => do_wrapping_addition::<U128, _>(cl_value, to_add),
        CLType::U256 => do_wrapping_addition::<U256, _>(cl_value, to_add),
        CLType::U512 => do_wrapping_addition::<U512, _>(cl_value, to_add),
        other => {
            let expected = format!("integral type compatible with {}", any::type_name::<Y>());
            let found = format!("{:?}", other);
            Err(TypeMismatch::new(expected, found).into())
        }
    }
}

/// Attempts a wrapping addition of `to_add` to the value represented by `cl_value`.
fn do_wrapping_addition<X, Y>(cl_value: CLValue, to_add: Y) -> Result<StoredValue, Error>
where
    X: WrappingAdd + CLTyped + ToBytes + FromBytes + Copy + 'static,
    Y: AsPrimitive<X>,
{
    let x: X = cl_value.into_t()?;
    let result = x.wrapping_add(&(to_add.as_()));
    Ok(StoredValue::CLValue(CLValue::from_t(result)?))
}

impl Transform {
    pub fn apply(self, stored_value: StoredValue) -> Result<StoredValue, Error> {
        match self {
            Transform::Identity => Ok(stored_value),
            Transform::Write(new_value) => Ok(new_value),
            Transform::AddInt32(to_add) => wrapping_addition(stored_value, to_add),
            Transform::AddUInt64(to_add) => wrapping_addition(stored_value, to_add),
            Transform::AddUInt128(to_add) => wrapping_addition(stored_value, to_add),
            Transform::AddUInt256(to_add) => wrapping_addition(stored_value, to_add),
            Transform::AddUInt512(to_add) => wrapping_addition(stored_value, to_add),
            Transform::AddKeys(mut keys) => match stored_value {
                StoredValue::Contract(mut contract) => {
                    contract.named_keys_append(&mut keys);
                    Ok(StoredValue::Contract(contract))
                }
                StoredValue::Account(mut account) => {
                    account.named_keys_append(&mut keys);
                    Ok(StoredValue::Account(account))
                }
                StoredValue::CLValue(cl_value) => {
                    let expected = "Contract or Account".to_string();
                    let found = format!("{:?}", cl_value.cl_type());
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::ContractPackage(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "ContractPackage".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::ContractWasm(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "ContractWasm".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::Transfer(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "Transfer".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::DeployInfo(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "DeployInfo".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::EraInfo(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "EraInfo".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::Bid(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "Bid".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
                StoredValue::Withdraw(_) => {
                    let expected = "Contract or Account".to_string();
                    let found = "Withdraw".to_string();
                    Err(TypeMismatch::new(expected, found).into())
                }
            },
            Transform::Failure(error) => Err(error),
        }
    }
}

/// Combines numeric `Transform`s into a single `Transform`. This is done by unwrapping the
/// `Transform` to obtain the underlying value, performing the wrapping addition then wrapping up as
/// a `Transform` again.
fn wrapped_transform_addition<T>(i: T, b: Transform, expected: &str) -> Transform
where
    T: WrappingAdd
        + AsPrimitive<i32>
        + From<u32>
        + From<u64>
        + Into<Transform>
        + TryFrom<Transform, Error = String>,
    i32: AsPrimitive<T>,
{
    if let Transform::AddInt32(j) = b {
        i.wrapping_add(&j.as_()).into()
    } else if let Transform::AddUInt64(j) = b {
        i.wrapping_add(&j.into()).into()
    } else {
        match T::try_from(b) {
            Err(b_type) => Transform::Failure(
                TypeMismatch {
                    expected: String::from(expected),
                    found: b_type,
                }
                .into(),
            ),

            Ok(j) => i.wrapping_add(&j).into(),
        }
    }
}

impl Add for Transform {
    type Output = Transform;

    fn add(self, other: Transform) -> Transform {
        match (self, other) {
            (a, Transform::Identity) => a,
            (Transform::Identity, b) => b,
            (a @ Transform::Failure(_), _) => a,
            (_, b @ Transform::Failure(_)) => b,
            (_, b @ Transform::Write(_)) => b,
            (Transform::Write(v), b) => {
                // second transform changes value being written
                match b.apply(v) {
                    Err(error) => Transform::Failure(error),
                    Ok(new_value) => Transform::Write(new_value),
                }
            }
            (Transform::AddInt32(i), b) => match b {
                Transform::AddInt32(j) => Transform::AddInt32(i.wrapping_add(j)),
                Transform::AddUInt64(j) => Transform::AddUInt64(j.wrapping_add(i as u64)),
                Transform::AddUInt128(j) => Transform::AddUInt128(j.wrapping_add(&(i.as_()))),
                Transform::AddUInt256(j) => Transform::AddUInt256(j.wrapping_add(&(i.as_()))),
                Transform::AddUInt512(j) => Transform::AddUInt512(j.wrapping_add(&i.as_())),
                other => Transform::Failure(
                    TypeMismatch::new("AddInt32".to_owned(), format!("{:?}", other)).into(),
                ),
            },
            (Transform::AddUInt64(i), b) => match b {
                Transform::AddInt32(j) => Transform::AddInt32(j.wrapping_add(i as i32)),
                Transform::AddUInt64(j) => Transform::AddUInt64(i.wrapping_add(j)),
                Transform::AddUInt128(j) => Transform::AddUInt128(j.wrapping_add(&i.into())),
                Transform::AddUInt256(j) => Transform::AddUInt256(j.wrapping_add(&i.into())),
                Transform::AddUInt512(j) => Transform::AddUInt512(j.wrapping_add(&i.into())),
                other => Transform::Failure(
                    TypeMismatch::new("AddUInt64".to_owned(), format!("{:?}", other)).into(),
                ),
            },
            (Transform::AddUInt128(i), b) => wrapped_transform_addition(i, b, "U128"),
            (Transform::AddUInt256(i), b) => wrapped_transform_addition(i, b, "U256"),
            (Transform::AddUInt512(i), b) => wrapped_transform_addition(i, b, "U512"),
            (Transform::AddKeys(mut ks1), b) => match b {
                Transform::AddKeys(mut ks2) => {
                    ks1.append(&mut ks2);
                    Transform::AddKeys(ks1)
                }
                other => Transform::Failure(
                    TypeMismatch::new("AddKeys".to_owned(), format!("{:?}", other)).into(),
                ),
            },
        }
    }
}

impl AddAssign for Transform {
    fn add_assign(&mut self, other: Self) {
        *self = self.clone() + other;
    }
}

impl Display for Transform {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Transform::Identity
    }
}

impl From<&Transform> for casper_types::Transform {
    fn from(transform: &Transform) -> Self {
        match transform {
            Transform::Identity => casper_types::Transform::Identity,
            Transform::Write(StoredValue::CLValue(cl_value)) => {
                casper_types::Transform::WriteCLValue(cl_value.clone())
            }
            Transform::Write(StoredValue::Account(account)) => {
                casper_types::Transform::WriteAccount(account.account_hash())
            }
            Transform::Write(StoredValue::ContractWasm(_)) => {
                casper_types::Transform::WriteContractWasm
            }
            Transform::Write(StoredValue::Contract(_)) => casper_types::Transform::WriteContract,
            Transform::Write(StoredValue::ContractPackage(_)) => {
                casper_types::Transform::WriteContractPackage
            }
            Transform::Write(StoredValue::Transfer(transfer)) => {
                casper_types::Transform::WriteTransfer(*transfer)
            }
            Transform::Write(StoredValue::DeployInfo(deploy_info)) => {
                casper_types::Transform::WriteDeployInfo(deploy_info.clone())
            }
            Transform::Write(StoredValue::EraInfo(era_info)) => {
                casper_types::Transform::WriteEraInfo(era_info.clone())
            }
            Transform::Write(StoredValue::Bid(bid)) => {
                casper_types::Transform::WriteBid(bid.clone())
            }
            Transform::Write(StoredValue::Withdraw(unbonding_purses)) => {
                casper_types::Transform::WriteWithdraw(unbonding_purses.clone())
            }
            Transform::AddInt32(value) => casper_types::Transform::AddInt32(*value),
            Transform::AddUInt64(value) => casper_types::Transform::AddUInt64(*value),
            Transform::AddUInt128(value) => casper_types::Transform::AddUInt128(*value),
            Transform::AddUInt256(value) => casper_types::Transform::AddUInt256(*value),
            Transform::AddUInt512(value) => casper_types::Transform::AddUInt512(*value),
            Transform::AddKeys(named_keys) => casper_types::Transform::AddKeys(
                named_keys
                    .iter()
                    .map(|(name, key)| casper_types::NamedKey {
                        name: name.clone(),
                        key: key.to_formatted_string(),
                    })
                    .collect(),
            ),
            Transform::Failure(error) => casper_types::Transform::Failure(error.to_string()),
        }
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{collection::vec, prelude::*};

    use super::Transform;
    use crate::shared::stored_value::gens::stored_value_arb;

    pub fn transform_arb() -> impl Strategy<Value = Transform> {
        prop_oneof![
            Just(Transform::Identity),
            stored_value_arb().prop_map(Transform::Write),
            any::<i32>().prop_map(Transform::AddInt32),
            any::<u64>().prop_map(Transform::AddUInt64),
            any::<u128>().prop_map(|u| Transform::AddUInt128(u.into())),
            vec(any::<u8>(), 32).prop_map(|u| {
                let mut buf: [u8; 32] = [0u8; 32];
                buf.copy_from_slice(&u);
                Transform::AddUInt256(buf.into())
            }),
            vec(any::<u8>(), 64).prop_map(|u| {
                let mut buf: [u8; 64] = [0u8; 64];
                buf.copy_from_slice(&u);
                Transform::AddUInt512(buf.into())
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use num::{Bounded, Num};

    use casper_types::{
        account::AccountHash, bytesrepr::Bytes, AccessRights, ContractWasm, Key, URef, U128, U256,
        U512,
    };

    use super::*;
    use crate::shared::account::{Account, ActionThresholds, AssociatedKeys};
    use std::collections::BTreeMap;

    const ZERO_ARRAY: [u8; 32] = [0; 32];
    const ZERO_PUBLIC_KEY: AccountHash = AccountHash::new(ZERO_ARRAY);
    const TEST_STR: &str = "a";
    const TEST_BOOL: bool = true;

    const ZERO_I32: i32 = 0;
    const ONE_I32: i32 = 1;
    const NEG_ONE_I32: i32 = -1;
    const NEG_TWO_I32: i32 = -2;
    const MIN_I32: i32 = i32::min_value();
    const MAX_I32: i32 = i32::max_value();

    const ZERO_I64: i64 = 0;
    const ONE_I64: i64 = 1;
    const NEG_ONE_I64: i64 = -1;
    const NEG_TWO_I64: i64 = -2;
    const MIN_I64: i64 = i64::min_value();
    const MAX_I64: i64 = i64::max_value();

    const ZERO_U8: u8 = 0;
    const ONE_U8: u8 = 1;
    const MAX_U8: u8 = u8::max_value();

    const ZERO_U32: u32 = 0;
    const ONE_U32: u32 = 1;
    const MAX_U32: u32 = u32::max_value();

    const ZERO_U64: u64 = 0;
    const ONE_U64: u64 = 1;
    const MAX_U64: u64 = u64::max_value();

    const ZERO_U128: U128 = U128([0; 2]);
    const ONE_U128: U128 = U128([1, 0]);
    const MAX_U128: U128 = U128([MAX_U64; 2]);

    const ZERO_U256: U256 = U256([0; 4]);
    const ONE_U256: U256 = U256([1, 0, 0, 0]);
    const MAX_U256: U256 = U256([MAX_U64; 4]);

    const ZERO_U512: U512 = U512([0; 8]);
    const ONE_U512: U512 = U512([1, 0, 0, 0, 0, 0, 0, 0]);
    const MAX_U512: U512 = U512([MAX_U64; 8]);

    #[test]
    fn i32_overflow() {
        let max = std::i32::MAX;
        let min = std::i32::MIN;

        let max_value = StoredValue::CLValue(CLValue::from_t(max).unwrap());
        let min_value = StoredValue::CLValue(CLValue::from_t(min).unwrap());

        let apply_overflow = Transform::AddInt32(1).apply(max_value.clone());
        let apply_underflow = Transform::AddInt32(-1).apply(min_value.clone());

        let transform_overflow = Transform::AddInt32(max) + Transform::AddInt32(1);
        let transform_underflow = Transform::AddInt32(min) + Transform::AddInt32(-1);

        assert_eq!(apply_overflow.expect("Unexpected overflow"), min_value);
        assert_eq!(apply_underflow.expect("Unexpected underflow"), max_value);

        assert_eq!(transform_overflow, min.into());
        assert_eq!(transform_underflow, max.into());
    }

    fn uint_overflow_test<T>()
    where
        T: Num + Bounded + CLTyped + ToBytes + Into<Transform> + Copy,
    {
        let max = T::max_value();
        let min = T::min_value();
        let one = T::one();
        let zero = T::zero();

        let max_value = StoredValue::CLValue(CLValue::from_t(max).unwrap());
        let min_value = StoredValue::CLValue(CLValue::from_t(min).unwrap());
        let zero_value = StoredValue::CLValue(CLValue::from_t(zero).unwrap());

        let max_transform: Transform = max.into();
        let min_transform: Transform = min.into();

        let one_transform: Transform = one.into();

        let apply_overflow = Transform::AddInt32(1).apply(max_value.clone());

        let apply_overflow_uint = one_transform.clone().apply(max_value.clone());
        let apply_underflow = Transform::AddInt32(-1).apply(min_value);

        let transform_overflow = max_transform.clone() + Transform::AddInt32(1);
        let transform_overflow_uint = max_transform + one_transform;
        let transform_underflow = min_transform + Transform::AddInt32(-1);

        assert_eq!(apply_overflow, Ok(zero_value.clone()));
        assert_eq!(apply_overflow_uint, Ok(zero_value));
        assert_eq!(apply_underflow, Ok(max_value));

        assert_eq!(transform_overflow, zero.into());
        assert_eq!(transform_overflow_uint, zero.into());
        assert_eq!(transform_underflow, max.into());
    }

    #[test]
    fn u128_overflow() {
        uint_overflow_test::<U128>();
    }

    #[test]
    fn u256_overflow() {
        uint_overflow_test::<U256>();
    }

    #[test]
    fn u512_overflow() {
        uint_overflow_test::<U512>();
    }

    #[test]
    fn addition_between_mismatched_types_should_fail() {
        fn assert_yields_type_mismatch_error(stored_value: StoredValue) {
            match wrapping_addition(stored_value, ZERO_I32) {
                Err(Error::TypeMismatch(_)) => (),
                _ => panic!("wrapping addition should yield TypeMismatch error"),
            };
        }

        let contract = StoredValue::ContractWasm(ContractWasm::new(vec![]));
        assert_yields_type_mismatch_error(contract);

        let uref = URef::new(ZERO_ARRAY, AccessRights::READ);
        let account = StoredValue::Account(Account::new(
            ZERO_PUBLIC_KEY,
            NamedKeys::new(),
            uref,
            AssociatedKeys::default(),
            ActionThresholds::default(),
        ));
        assert_yields_type_mismatch_error(account);

        let cl_bool =
            StoredValue::CLValue(CLValue::from_t(TEST_BOOL).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_bool);

        let cl_unit = StoredValue::CLValue(CLValue::from_t(()).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_unit);

        let cl_string =
            StoredValue::CLValue(CLValue::from_t(TEST_STR).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_string);

        let cl_key = StoredValue::CLValue(
            CLValue::from_t(Key::Hash(ZERO_ARRAY)).expect("should create CLValue"),
        );
        assert_yields_type_mismatch_error(cl_key);

        let cl_uref = StoredValue::CLValue(CLValue::from_t(uref).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_uref);

        let cl_option =
            StoredValue::CLValue(CLValue::from_t(Some(ZERO_U8)).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_option);

        let cl_list = StoredValue::CLValue(
            CLValue::from_t(Bytes::from(vec![ZERO_U8])).expect("should create CLValue"),
        );
        assert_yields_type_mismatch_error(cl_list);

        let cl_fixed_list =
            StoredValue::CLValue(CLValue::from_t([ZERO_U8]).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_fixed_list);

        let cl_result: Result<(), u8> = Err(ZERO_U8);
        let cl_result =
            StoredValue::CLValue(CLValue::from_t(cl_result).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_result);

        let cl_map = StoredValue::CLValue(
            CLValue::from_t(BTreeMap::<u8, u8>::new()).expect("should create CLValue"),
        );
        assert_yields_type_mismatch_error(cl_map);

        let cl_tuple1 =
            StoredValue::CLValue(CLValue::from_t((ZERO_U8,)).expect("should create CLValue"));
        assert_yields_type_mismatch_error(cl_tuple1);

        let cl_tuple2 = StoredValue::CLValue(
            CLValue::from_t((ZERO_U8, ZERO_U8)).expect("should create CLValue"),
        );
        assert_yields_type_mismatch_error(cl_tuple2);

        let cl_tuple3 = StoredValue::CLValue(
            CLValue::from_t((ZERO_U8, ZERO_U8, ZERO_U8)).expect("should create CLValue"),
        );
        assert_yields_type_mismatch_error(cl_tuple3);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn wrapping_addition_should_succeed() {
        fn add<X, Y>(current_value: X, to_add: Y) -> X
        where
            X: CLTyped + ToBytes + FromBytes + PartialEq + fmt::Debug,
            Y: AsPrimitive<i32>
                + AsPrimitive<i64>
                + AsPrimitive<u8>
                + AsPrimitive<u32>
                + AsPrimitive<u64>
                + AsPrimitive<U128>
                + AsPrimitive<U256>
                + AsPrimitive<U512>,
        {
            let current = StoredValue::CLValue(
                CLValue::from_t(current_value).expect("should create CLValue"),
            );
            let result =
                wrapping_addition(current, to_add).expect("wrapping addition should succeed");
            CLValue::try_from(result)
                .expect("should be CLValue")
                .into_t()
                .expect("should parse to X")
        }

        // Adding to i32
        assert_eq!(ONE_I32, add(ZERO_I32, ONE_I32));
        assert_eq!(MIN_I32, add(MAX_I32, ONE_I32));
        assert_eq!(NEG_TWO_I32, add(MAX_I32, MAX_I32));
        assert_eq!(ZERO_I32, add(ONE_I32, NEG_ONE_I32));
        assert_eq!(NEG_ONE_I32, add(ZERO_I32, NEG_ONE_I32));
        assert_eq!(MAX_I32, add(NEG_ONE_I32, MIN_I32));

        assert_eq!(ONE_I32, add(ZERO_I32, ONE_U64));
        assert_eq!(MIN_I32, add(MAX_I32, ONE_U64));
        assert_eq!(NEG_TWO_I32, add(MAX_I32, MAX_I32 as u64));

        assert_eq!(ONE_I32, add(ZERO_I32, ONE_U128));
        assert_eq!(MIN_I32, add(MAX_I32, ONE_U128));
        assert_eq!(NEG_TWO_I32, add(MAX_I32, U128::from(MAX_I32)));

        assert_eq!(ONE_I32, add(ZERO_I32, ONE_U256));
        assert_eq!(MIN_I32, add(MAX_I32, ONE_U256));
        assert_eq!(NEG_TWO_I32, add(MAX_I32, U256::from(MAX_I32)));

        assert_eq!(ONE_I32, add(ZERO_I32, ONE_U512));
        assert_eq!(MIN_I32, add(MAX_I32, ONE_U512));
        assert_eq!(NEG_TWO_I32, add(MAX_I32, U512::from(MAX_I32)));

        // Adding to i64
        assert_eq!(ONE_I64, add(ZERO_I64, ONE_I32));
        assert_eq!(MIN_I64, add(MAX_I64, ONE_I32));
        assert_eq!(ZERO_I64, add(ONE_I64, NEG_ONE_I32));
        assert_eq!(NEG_ONE_I64, add(ZERO_I64, NEG_ONE_I32));
        assert_eq!(MAX_I64, add(MIN_I64, NEG_ONE_I32));

        assert_eq!(ONE_I64, add(ZERO_I64, ONE_U64));
        assert_eq!(MIN_I64, add(MAX_I64, ONE_U64));
        assert_eq!(NEG_TWO_I64, add(MAX_I64, MAX_I64 as u64));

        assert_eq!(ONE_I64, add(ZERO_I64, ONE_U128));
        assert_eq!(MIN_I64, add(MAX_I64, ONE_U128));
        assert_eq!(NEG_TWO_I64, add(MAX_I64, U128::from(MAX_I64)));

        assert_eq!(ONE_I64, add(ZERO_I64, ONE_U256));
        assert_eq!(MIN_I64, add(MAX_I64, ONE_U256));
        assert_eq!(NEG_TWO_I64, add(MAX_I64, U256::from(MAX_I64)));

        assert_eq!(ONE_I64, add(ZERO_I64, ONE_U512));
        assert_eq!(MIN_I64, add(MAX_I64, ONE_U512));
        assert_eq!(NEG_TWO_I64, add(MAX_I64, U512::from(MAX_I64)));

        // Adding to u8
        assert_eq!(ONE_U8, add(ZERO_U8, ONE_I32));
        assert_eq!(ZERO_U8, add(MAX_U8, ONE_I32));
        assert_eq!(MAX_U8, add(MAX_U8, 256_i32));
        assert_eq!(ZERO_U8, add(MAX_U8, 257_i32));
        assert_eq!(ZERO_U8, add(ONE_U8, NEG_ONE_I32));
        assert_eq!(MAX_U8, add(ZERO_U8, NEG_ONE_I32));
        assert_eq!(ZERO_U8, add(ZERO_U8, -256_i32));
        assert_eq!(MAX_U8, add(ZERO_U8, -257_i32));
        assert_eq!(MAX_U8, add(ZERO_U8, MAX_I32));
        assert_eq!(ZERO_U8, add(ZERO_U8, MIN_I32));

        assert_eq!(ONE_U8, add(ZERO_U8, ONE_U64));
        assert_eq!(ZERO_U8, add(MAX_U8, ONE_U64));
        assert_eq!(ONE_U8, add(ZERO_U8, u64::from(MAX_U8) + 2));
        assert_eq!(MAX_U8, add(ZERO_U8, MAX_U64));

        assert_eq!(ONE_U8, add(ZERO_U8, ONE_U128));
        assert_eq!(ZERO_U8, add(MAX_U8, ONE_U128));
        assert_eq!(ONE_U8, add(ZERO_U8, U128::from(MAX_U8) + 2));
        assert_eq!(MAX_U8, add(ZERO_U8, MAX_U128));

        assert_eq!(ONE_U8, add(ZERO_U8, ONE_U256));
        assert_eq!(ZERO_U8, add(MAX_U8, ONE_U256));
        assert_eq!(ONE_U8, add(ZERO_U8, U256::from(MAX_U8) + 2));
        assert_eq!(MAX_U8, add(ZERO_U8, MAX_U256));

        assert_eq!(ONE_U8, add(ZERO_U8, ONE_U512));
        assert_eq!(ZERO_U8, add(MAX_U8, ONE_U512));
        assert_eq!(ONE_U8, add(ZERO_U8, U512::from(MAX_U8) + 2));
        assert_eq!(MAX_U8, add(ZERO_U8, MAX_U512));

        // Adding to u32
        assert_eq!(ONE_U32, add(ZERO_U32, ONE_I32));
        assert_eq!(ZERO_U32, add(MAX_U32, ONE_I32));
        assert_eq!(ZERO_U32, add(ONE_U32, NEG_ONE_I32));
        assert_eq!(MAX_U32, add(ZERO_U32, NEG_ONE_I32));
        assert_eq!(MAX_I32 as u32 + 1, add(ZERO_U32, MIN_I32));

        assert_eq!(ONE_U32, add(ZERO_U32, ONE_U64));
        assert_eq!(ZERO_U32, add(MAX_U32, ONE_U64));
        assert_eq!(ONE_U32, add(ZERO_U32, u64::from(MAX_U32) + 2));
        assert_eq!(MAX_U32, add(ZERO_U32, MAX_U64));

        assert_eq!(ONE_U32, add(ZERO_U32, ONE_U128));
        assert_eq!(ZERO_U32, add(MAX_U32, ONE_U128));
        assert_eq!(ONE_U32, add(ZERO_U32, U128::from(MAX_U32) + 2));
        assert_eq!(MAX_U32, add(ZERO_U32, MAX_U128));

        assert_eq!(ONE_U32, add(ZERO_U32, ONE_U256));
        assert_eq!(ZERO_U32, add(MAX_U32, ONE_U256));
        assert_eq!(ONE_U32, add(ZERO_U32, U256::from(MAX_U32) + 2));
        assert_eq!(MAX_U32, add(ZERO_U32, MAX_U256));

        assert_eq!(ONE_U32, add(ZERO_U32, ONE_U512));
        assert_eq!(ZERO_U32, add(MAX_U32, ONE_U512));
        assert_eq!(ONE_U32, add(ZERO_U32, U512::from(MAX_U32) + 2));
        assert_eq!(MAX_U32, add(ZERO_U32, MAX_U512));

        // Adding to u64
        assert_eq!(ONE_U64, add(ZERO_U64, ONE_I32));
        assert_eq!(ZERO_U64, add(MAX_U64, ONE_I32));
        assert_eq!(ZERO_U64, add(ONE_U64, NEG_ONE_I32));
        assert_eq!(MAX_U64, add(ZERO_U64, NEG_ONE_I32));

        assert_eq!(ONE_U64, add(ZERO_U64, ONE_U64));
        assert_eq!(ZERO_U64, add(MAX_U64, ONE_U64));
        assert_eq!(MAX_U64 - 1, add(MAX_U64, MAX_U64));

        assert_eq!(ONE_U64, add(ZERO_U64, ONE_U128));
        assert_eq!(ZERO_U64, add(MAX_U64, ONE_U128));
        assert_eq!(ONE_U64, add(ZERO_U64, U128::from(MAX_U64) + 2));
        assert_eq!(MAX_U64, add(ZERO_U64, MAX_U128));

        assert_eq!(ONE_U64, add(ZERO_U64, ONE_U256));
        assert_eq!(ZERO_U64, add(MAX_U64, ONE_U256));
        assert_eq!(ONE_U64, add(ZERO_U64, U256::from(MAX_U64) + 2));
        assert_eq!(MAX_U64, add(ZERO_U64, MAX_U256));

        assert_eq!(ONE_U64, add(ZERO_U64, ONE_U512));
        assert_eq!(ZERO_U64, add(MAX_U64, ONE_U512));
        assert_eq!(ONE_U64, add(ZERO_U64, U512::from(MAX_U64) + 2));
        assert_eq!(MAX_U64, add(ZERO_U64, MAX_U512));

        // Adding to U128
        assert_eq!(ONE_U128, add(ZERO_U128, ONE_I32));
        assert_eq!(ZERO_U128, add(MAX_U128, ONE_I32));
        assert_eq!(ZERO_U128, add(ONE_U128, NEG_ONE_I32));
        assert_eq!(MAX_U128, add(ZERO_U128, NEG_ONE_I32));

        assert_eq!(ONE_U128, add(ZERO_U128, ONE_U64));
        assert_eq!(ZERO_U128, add(MAX_U128, ONE_U64));

        assert_eq!(ONE_U128, add(ZERO_U128, ONE_U128));
        assert_eq!(ZERO_U128, add(MAX_U128, ONE_U128));
        assert_eq!(MAX_U128 - 1, add(MAX_U128, MAX_U128));

        assert_eq!(ONE_U128, add(ZERO_U128, ONE_U256));
        assert_eq!(ZERO_U128, add(MAX_U128, ONE_U256));
        assert_eq!(
            ONE_U128,
            add(
                ZERO_U128,
                U256::from_dec_str(&MAX_U128.to_string()).unwrap() + 2,
            )
        );
        assert_eq!(MAX_U128, add(ZERO_U128, MAX_U256));

        assert_eq!(ONE_U128, add(ZERO_U128, ONE_U512));
        assert_eq!(ZERO_U128, add(MAX_U128, ONE_U512));
        assert_eq!(
            ONE_U128,
            add(
                ZERO_U128,
                U512::from_dec_str(&MAX_U128.to_string()).unwrap() + 2,
            )
        );
        assert_eq!(MAX_U128, add(ZERO_U128, MAX_U512));

        // Adding to U256
        assert_eq!(ONE_U256, add(ZERO_U256, ONE_I32));
        assert_eq!(ZERO_U256, add(MAX_U256, ONE_I32));
        assert_eq!(ZERO_U256, add(ONE_U256, NEG_ONE_I32));
        assert_eq!(MAX_U256, add(ZERO_U256, NEG_ONE_I32));

        assert_eq!(ONE_U256, add(ZERO_U256, ONE_U64));
        assert_eq!(ZERO_U256, add(MAX_U256, ONE_U64));

        assert_eq!(ONE_U256, add(ZERO_U256, ONE_U128));
        assert_eq!(ZERO_U256, add(MAX_U256, ONE_U128));

        assert_eq!(ONE_U256, add(ZERO_U256, ONE_U256));
        assert_eq!(ZERO_U256, add(MAX_U256, ONE_U256));
        assert_eq!(MAX_U256 - 1, add(MAX_U256, MAX_U256));

        assert_eq!(ONE_U256, add(ZERO_U256, ONE_U512));
        assert_eq!(ZERO_U256, add(MAX_U256, ONE_U512));
        assert_eq!(
            ONE_U256,
            add(
                ZERO_U256,
                U512::from_dec_str(&MAX_U256.to_string()).unwrap() + 2,
            )
        );
        assert_eq!(MAX_U256, add(ZERO_U256, MAX_U512));

        // Adding to U512
        assert_eq!(ONE_U512, add(ZERO_U512, ONE_I32));
        assert_eq!(ZERO_U512, add(MAX_U512, ONE_I32));
        assert_eq!(ZERO_U512, add(ONE_U512, NEG_ONE_I32));
        assert_eq!(MAX_U512, add(ZERO_U512, NEG_ONE_I32));

        assert_eq!(ONE_U512, add(ZERO_U512, ONE_U64));
        assert_eq!(ZERO_U512, add(MAX_U512, ONE_U64));

        assert_eq!(ONE_U512, add(ZERO_U512, ONE_U128));
        assert_eq!(ZERO_U512, add(MAX_U512, ONE_U128));

        assert_eq!(ONE_U512, add(ZERO_U512, ONE_U256));
        assert_eq!(ZERO_U512, add(MAX_U512, ONE_U256));

        assert_eq!(ONE_U512, add(ZERO_U512, ONE_U512));
        assert_eq!(ZERO_U512, add(MAX_U512, ONE_U512));
        assert_eq!(MAX_U512 - 1, add(MAX_U512, MAX_U512));
    }
}
