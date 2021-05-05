// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    string::String,
    vec::Vec,
};
use core::mem;

use num_derive::{FromPrimitive, ToPrimitive};
use num_rational::Ratio;
use num_traits::{FromPrimitive, ToPrimitive};
#[cfg(feature = "std")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    bytesrepr::{self, FromBytes, ToBytes},
    Key, URef, U128, U256, U512,
};

#[derive(FromPrimitive, ToPrimitive)]
#[repr(u8)]
enum CLTypeTag {
    Bool = 0,
    I32 = 1,
    I64 = 2,
    U8 = 3,
    U32 = 4,
    U64 = 5,
    U128 = 6,
    U256 = 7,
    U512 = 8,
    Unit = 9,
    String = 10,
    Key = 11,
    URef = 12,
    Option = 13,
    List = 14,
    ByteArray = 15,
    Result = 16,
    Map = 17,
    Tuple1 = 18,
    Tuple2 = 19,
    Tuple3 = 20,
    Any = 21,
    PublicKey = 22,
}

impl From<CLTypeTag> for u8 {
    fn from(tag: CLTypeTag) -> Self {
        tag.to_u8().expect("CLTypeTag is represented as u8")
    }
}

impl FromBytes for CLTypeTag {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag_value, rem) = FromBytes::from_bytes(bytes)?;
        let tag = CLTypeTag::from_u8(tag_value).ok_or(bytesrepr::Error::Formatting)?;
        Ok((tag, rem))
    }
}

/// Casper types, i.e. types which can be stored and manipulated by smart contracts.
///
/// Provides a description of the underlying data type of a [`CLValue`](crate::CLValue).
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub enum CLType {
    /// `bool` primitive.
    Bool,
    /// `i32` primitive.
    I32,
    /// `i64` primitive.
    I64,
    /// `u8` primitive.
    U8,
    /// `u32` primitive.
    U32,
    /// `u64` primitive.
    U64,
    /// [`U128`] large unsigned integer type.
    U128,
    /// [`U256`] large unsigned integer type.
    U256,
    /// [`U512`] large unsigned integer type.
    U512,
    /// `()` primitive.
    Unit,
    /// `String` primitive.
    String,
    /// [`Key`] system type.
    Key,
    /// [`URef`] system type.
    URef,
    /// [`PublicKey`](crate::PublicKey) system type.
    PublicKey,
    /// `Option` of a `CLType`.
    Option(Box<CLType>),
    /// Variable-length list of a single `CLType` (comparable to a `Vec`).
    List(Box<CLType>),
    /// Fixed-length list of a single `CLType` (comparable to a Rust array).
    ByteArray(u32),
    /// `Result` with `Ok` and `Err` variants of `CLType`s.
    #[allow(missing_docs)] // generated docs are explicit enough.
    Result { ok: Box<CLType>, err: Box<CLType> },
    /// Map with keys of a single `CLType` and values of a single `CLType`.
    #[allow(missing_docs)] // generated docs are explicit enough.
    Map {
        key: Box<CLType>,
        value: Box<CLType>,
    },
    /// 1-ary tuple of a `CLType`.
    Tuple1([Box<CLType>; 1]),
    /// 2-ary tuple of `CLType`s.
    Tuple2([Box<CLType>; 2]),
    /// 3-ary tuple of `CLType`s.
    Tuple3([Box<CLType>; 3]),
    /// Unspecified type.
    Any,
}

impl CLType {
    /// The `len()` of the `Vec<u8>` resulting from `self.to_bytes()`.
    pub fn serialized_length(&self) -> usize {
        mem::size_of::<u8>()
            + match self {
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
                | CLType::Any => 0,
                CLType::Option(cl_type) | CLType::List(cl_type) => cl_type.serialized_length(),
                CLType::ByteArray(list_len) => list_len.serialized_length(),
                CLType::Result { ok, err } => ok.serialized_length() + err.serialized_length(),
                CLType::Map { key, value } => key.serialized_length() + value.serialized_length(),
                CLType::Tuple1(cl_type_array) => serialized_length_of_cl_tuple_type(cl_type_array),
                CLType::Tuple2(cl_type_array) => serialized_length_of_cl_tuple_type(cl_type_array),
                CLType::Tuple3(cl_type_array) => serialized_length_of_cl_tuple_type(cl_type_array),
            }
    }
}

/// Returns the `CLType` describing a "named key" on the system, i.e. a `(String, Key)`.
pub fn named_key_type() -> CLType {
    CLType::Tuple2([Box::new(CLType::String), Box::new(CLType::Key)])
}

impl CLType {
    pub(crate) fn append_bytes(&self, stream: &mut Vec<u8>) -> Result<(), bytesrepr::Error> {
        match self {
            CLType::Bool => stream.push(CLTypeTag::Bool.into()),
            CLType::I32 => stream.push(CLTypeTag::I32.into()),
            CLType::I64 => stream.push(CLTypeTag::I64.into()),
            CLType::U8 => stream.push(CLTypeTag::U8.into()),
            CLType::U32 => stream.push(CLTypeTag::U32.into()),
            CLType::U64 => stream.push(CLTypeTag::U64.into()),
            CLType::U128 => stream.push(CLTypeTag::U128.into()),
            CLType::U256 => stream.push(CLTypeTag::U256.into()),
            CLType::U512 => stream.push(CLTypeTag::U512.into()),
            CLType::Unit => stream.push(CLTypeTag::Unit.into()),
            CLType::String => stream.push(CLTypeTag::String.into()),
            CLType::Key => stream.push(CLTypeTag::Key.into()),
            CLType::URef => stream.push(CLTypeTag::URef.into()),
            CLType::PublicKey => stream.push(CLTypeTag::PublicKey.into()),
            CLType::Option(cl_type) => {
                stream.push(CLTypeTag::Option.into());
                cl_type.append_bytes(stream)?;
            }
            CLType::List(cl_type) => {
                stream.push(CLTypeTag::List.into());
                cl_type.append_bytes(stream)?;
            }
            CLType::ByteArray(len) => {
                stream.push(CLTypeTag::ByteArray.into());
                stream.append(&mut len.to_bytes()?);
            }
            CLType::Result { ok, err } => {
                stream.push(CLTypeTag::Result.into());
                ok.append_bytes(stream)?;
                err.append_bytes(stream)?;
            }
            CLType::Map { key, value } => {
                stream.push(CLTypeTag::Map.into());
                key.append_bytes(stream)?;
                value.append_bytes(stream)?;
            }
            CLType::Tuple1(cl_type_array) => {
                serialize_cl_tuple_type(CLTypeTag::Tuple1, cl_type_array, stream)?
            }
            CLType::Tuple2(cl_type_array) => {
                serialize_cl_tuple_type(CLTypeTag::Tuple2, cl_type_array, stream)?
            }
            CLType::Tuple3(cl_type_array) => {
                serialize_cl_tuple_type(CLTypeTag::Tuple3, cl_type_array, stream)?
            }
            CLType::Any => stream.push(CLTypeTag::Any.into()),
        }
        Ok(())
    }
}

#[allow(clippy::cognitive_complexity)]
impl FromBytes for CLType {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = CLTypeTag::from_bytes(bytes)?;
        match tag {
            CLTypeTag::Bool => Ok((CLType::Bool, remainder)),
            CLTypeTag::I32 => Ok((CLType::I32, remainder)),
            CLTypeTag::I64 => Ok((CLType::I64, remainder)),
            CLTypeTag::U8 => Ok((CLType::U8, remainder)),
            CLTypeTag::U32 => Ok((CLType::U32, remainder)),
            CLTypeTag::U64 => Ok((CLType::U64, remainder)),
            CLTypeTag::U128 => Ok((CLType::U128, remainder)),
            CLTypeTag::U256 => Ok((CLType::U256, remainder)),
            CLTypeTag::U512 => Ok((CLType::U512, remainder)),
            CLTypeTag::Unit => Ok((CLType::Unit, remainder)),
            CLTypeTag::String => Ok((CLType::String, remainder)),
            CLTypeTag::Key => Ok((CLType::Key, remainder)),
            CLTypeTag::URef => Ok((CLType::URef, remainder)),
            CLTypeTag::PublicKey => Ok((CLType::PublicKey, remainder)),
            CLTypeTag::Option => {
                let (inner_type, remainder) = CLType::from_bytes(remainder)?;
                let cl_type = CLType::Option(Box::new(inner_type));
                Ok((cl_type, remainder))
            }
            CLTypeTag::List => {
                let (inner_type, remainder) = CLType::from_bytes(remainder)?;
                let cl_type = CLType::List(Box::new(inner_type));
                Ok((cl_type, remainder))
            }
            CLTypeTag::ByteArray => {
                let (len, remainder) = u32::from_bytes(remainder)?;
                let cl_type = CLType::ByteArray(len);
                Ok((cl_type, remainder))
            }
            CLTypeTag::Result => {
                let (ok_type, remainder) = CLType::from_bytes(remainder)?;
                let (err_type, remainder) = CLType::from_bytes(remainder)?;
                let cl_type = CLType::Result {
                    ok: Box::new(ok_type),
                    err: Box::new(err_type),
                };
                Ok((cl_type, remainder))
            }
            CLTypeTag::Map => {
                let (key_type, remainder) = CLType::from_bytes(remainder)?;
                let (value_type, remainder) = CLType::from_bytes(remainder)?;
                let cl_type = CLType::Map {
                    key: Box::new(key_type),
                    value: Box::new(value_type),
                };
                Ok((cl_type, remainder))
            }
            CLTypeTag::Tuple1 => {
                let (mut inner_types, remainder) = parse_cl_tuple_types(1, remainder)?;
                // NOTE: Assumed safe as `parse_cl_tuple_types` is expected to have exactly 1
                // element
                let cl_type = CLType::Tuple1([inner_types.pop_front().unwrap()]);
                Ok((cl_type, remainder))
            }
            CLTypeTag::Tuple2 => {
                let (mut inner_types, remainder) = parse_cl_tuple_types(2, remainder)?;
                // NOTE: Assumed safe as `parse_cl_tuple_types` is expected to have exactly 2
                // elements
                let cl_type = CLType::Tuple2([
                    inner_types.pop_front().unwrap(),
                    inner_types.pop_front().unwrap(),
                ]);
                Ok((cl_type, remainder))
            }
            CLTypeTag::Tuple3 => {
                let (mut inner_types, remainder) = parse_cl_tuple_types(3, remainder)?;
                // NOTE: Assumed safe as `parse_cl_tuple_types` is expected to have exactly 3
                // elements
                let cl_type = CLType::Tuple3([
                    inner_types.pop_front().unwrap(),
                    inner_types.pop_front().unwrap(),
                    inner_types.pop_front().unwrap(),
                ]);
                Ok((cl_type, remainder))
            }
            CLTypeTag::Any => Ok((CLType::Any, remainder)),
        }
    }
}

fn serialize_cl_tuple_type<'a, T: IntoIterator<Item = &'a Box<CLType>>>(
    tag: CLTypeTag,
    cl_type_array: T,
    stream: &mut Vec<u8>,
) -> Result<(), bytesrepr::Error> {
    stream.push(tag.into());
    for cl_type in cl_type_array {
        cl_type.append_bytes(stream)?;
    }
    Ok(())
}

fn parse_cl_tuple_types(
    count: usize,
    mut bytes: &[u8],
) -> Result<(VecDeque<Box<CLType>>, &[u8]), bytesrepr::Error> {
    let mut cl_types = VecDeque::with_capacity(count);
    for _ in 0..count {
        let (cl_type, remainder) = CLType::from_bytes(bytes)?;
        cl_types.push_back(Box::new(cl_type));
        bytes = remainder;
    }

    Ok((cl_types, bytes))
}

fn serialized_length_of_cl_tuple_type<'a, T: IntoIterator<Item = &'a Box<CLType>>>(
    cl_type_array: T,
) -> usize {
    cl_type_array
        .into_iter()
        .map(|cl_type| cl_type.serialized_length())
        .sum()
}

/// A type which can be described as a [`CLType`].
pub trait CLTyped {
    /// The `CLType` of `Self`.
    fn cl_type() -> CLType;
}

impl CLTyped for bool {
    fn cl_type() -> CLType {
        CLType::Bool
    }
}

impl CLTyped for i32 {
    fn cl_type() -> CLType {
        CLType::I32
    }
}

impl CLTyped for i64 {
    fn cl_type() -> CLType {
        CLType::I64
    }
}

impl CLTyped for u8 {
    fn cl_type() -> CLType {
        CLType::U8
    }
}

impl CLTyped for u32 {
    fn cl_type() -> CLType {
        CLType::U32
    }
}

impl CLTyped for u64 {
    fn cl_type() -> CLType {
        CLType::U64
    }
}

impl CLTyped for U128 {
    fn cl_type() -> CLType {
        CLType::U128
    }
}

impl CLTyped for U256 {
    fn cl_type() -> CLType {
        CLType::U256
    }
}

impl CLTyped for U512 {
    fn cl_type() -> CLType {
        CLType::U512
    }
}

impl CLTyped for () {
    fn cl_type() -> CLType {
        CLType::Unit
    }
}

impl CLTyped for String {
    fn cl_type() -> CLType {
        CLType::String
    }
}

impl CLTyped for &str {
    fn cl_type() -> CLType {
        CLType::String
    }
}

impl CLTyped for Key {
    fn cl_type() -> CLType {
        CLType::Key
    }
}

impl CLTyped for URef {
    fn cl_type() -> CLType {
        CLType::URef
    }
}

impl<T: CLTyped> CLTyped for Option<T> {
    fn cl_type() -> CLType {
        CLType::Option(Box::new(T::cl_type()))
    }
}

impl<T: CLTyped> CLTyped for Vec<T> {
    fn cl_type() -> CLType {
        CLType::List(Box::new(T::cl_type()))
    }
}

macro_rules! impl_cl_typed_for_array {
    ($($N:literal)+) => {
        $(
            impl CLTyped for [u8; $N] {
                fn cl_type() -> CLType {
                    CLType::ByteArray($N as u32)
                }
            }
        )+
    }
}

impl_cl_typed_for_array! {
      0  1  2  3  4  5  6  7  8  9
     10 11 12 13 14 15 16 17 18 19
     20 21 22 23 24 25 26 27 28 29
     30 31 32
     64 128 256 512
}

impl<T: CLTyped, E: CLTyped> CLTyped for Result<T, E> {
    fn cl_type() -> CLType {
        let ok = Box::new(T::cl_type());
        let err = Box::new(E::cl_type());
        CLType::Result { ok, err }
    }
}

impl<K: CLTyped, V: CLTyped> CLTyped for BTreeMap<K, V> {
    fn cl_type() -> CLType {
        let key = Box::new(K::cl_type());
        let value = Box::new(V::cl_type());
        CLType::Map { key, value }
    }
}

impl<T1: CLTyped> CLTyped for (T1,) {
    fn cl_type() -> CLType {
        CLType::Tuple1([Box::new(T1::cl_type())])
    }
}

impl<T1: CLTyped, T2: CLTyped> CLTyped for (T1, T2) {
    fn cl_type() -> CLType {
        CLType::Tuple2([Box::new(T1::cl_type()), Box::new(T2::cl_type())])
    }
}

impl<T1: CLTyped, T2: CLTyped, T3: CLTyped> CLTyped for (T1, T2, T3) {
    fn cl_type() -> CLType {
        CLType::Tuple3([
            Box::new(T1::cl_type()),
            Box::new(T2::cl_type()),
            Box::new(T3::cl_type()),
        ])
    }
}

impl<T: CLTyped> CLTyped for Ratio<T> {
    fn cl_type() -> CLType {
        <(T, T)>::cl_type()
    }
}

#[cfg(test)]
mod tests {
    use std::{fmt::Debug, string::ToString};

    use super::*;
    use crate::{
        bytesrepr::{FromBytes, ToBytes},
        AccessRights, CLValue,
    };

    fn round_trip<T: CLTyped + FromBytes + ToBytes + PartialEq + Debug + Clone>(value: &T) {
        let cl_value = CLValue::from_t(value.clone()).unwrap();

        let serialized_cl_value = cl_value.to_bytes().unwrap();
        assert_eq!(serialized_cl_value.len(), cl_value.serialized_length());
        let parsed_cl_value: CLValue = bytesrepr::deserialize(serialized_cl_value).unwrap();
        assert_eq!(cl_value, parsed_cl_value);

        let parsed_value = CLValue::into_t(cl_value).unwrap();
        assert_eq!(*value, parsed_value);
    }

    #[test]
    fn bool_should_work() {
        round_trip(&true);
        round_trip(&false);
    }

    #[test]
    fn u8_should_work() {
        round_trip(&1u8);
    }

    #[test]
    fn u32_should_work() {
        round_trip(&1u32);
    }

    #[test]
    fn i32_should_work() {
        round_trip(&-1i32);
    }

    #[test]
    fn u64_should_work() {
        round_trip(&1u64);
    }

    #[test]
    fn i64_should_work() {
        round_trip(&-1i64);
    }

    #[test]
    fn u128_should_work() {
        round_trip(&U128::one());
    }

    #[test]
    fn u256_should_work() {
        round_trip(&U256::one());
    }

    #[test]
    fn u512_should_work() {
        round_trip(&U512::one());
    }

    #[test]
    fn unit_should_work() {
        round_trip(&());
    }

    #[test]
    fn string_should_work() {
        round_trip(&String::from("abc"));
    }

    #[test]
    fn key_should_work() {
        let key = Key::URef(URef::new([0u8; 32], AccessRights::READ_ADD_WRITE));
        round_trip(&key);
    }

    #[test]
    fn uref_should_work() {
        let uref = URef::new([0u8; 32], AccessRights::READ_ADD_WRITE);
        round_trip(&uref);
    }

    #[test]
    fn option_of_cl_type_should_work() {
        let x: Option<i32> = Some(-1);
        let y: Option<i32> = None;

        round_trip(&x);
        round_trip(&y);
    }

    #[test]
    fn vec_of_cl_type_should_work() {
        let vec = vec![String::from("a"), String::from("b")];
        round_trip(&vec);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn small_array_of_u8_should_work() {
        macro_rules! test_small_array {
            ($($N:literal)+) => {
                $(
                    let mut array: [u8; $N] = Default::default();
                    for i in 0..$N {
                        array[i] = i as u8;
                    }
                    round_trip(&array);
                )+
            }
        }

        test_small_array! {
                 1  2  3  4  5  6  7  8  9
             10 11 12 13 14 15 16 17 18 19
             20 21 22 23 24 25 26 27 28 29
             30 31 32
        }
    }

    #[test]
    fn large_array_of_cl_type_should_work() {
        macro_rules! test_large_array {
            ($($N:literal)+) => {
                $(
                    let array = {
                        let mut tmp = [0u8; $N];
                        for i in 0..$N {
                            tmp[i] = i as u8;
                        }
                        tmp
                    };

                    let cl_value = CLValue::from_t(array.clone()).unwrap();

                    let serialized_cl_value = cl_value.to_bytes().unwrap();
                    let parsed_cl_value: CLValue = bytesrepr::deserialize(serialized_cl_value).unwrap();
                    assert_eq!(cl_value, parsed_cl_value);

                    let parsed_value: [u8; $N] = CLValue::into_t(cl_value).unwrap();
                    for i in 0..$N {
                        assert_eq!(array[i], parsed_value[i]);
                    }
                )+
            }
        }

        test_large_array! { 64 128 256 512 }
    }

    #[test]
    fn result_of_cl_type_should_work() {
        let x: Result<(), String> = Ok(());
        let y: Result<(), String> = Err(String::from("Hello, world!"));

        round_trip(&x);
        round_trip(&y);
    }

    #[test]
    fn map_of_cl_type_should_work() {
        let mut map: BTreeMap<String, u64> = BTreeMap::new();
        map.insert(String::from("abc"), 1);
        map.insert(String::from("xyz"), 2);

        round_trip(&map);
    }

    #[test]
    fn tuple_1_should_work() {
        let x = (-1i32,);

        round_trip(&x);
    }

    #[test]
    fn tuple_2_should_work() {
        let x = (-1i32, String::from("a"));

        round_trip(&x);
    }

    #[test]
    fn tuple_3_should_work() {
        let x = (-1i32, 1u32, String::from("a"));

        round_trip(&x);
    }

    #[test]
    fn any_should_work() {
        #[derive(PartialEq, Debug, Clone)]
        struct Any(String);

        impl CLTyped for Any {
            fn cl_type() -> CLType {
                CLType::Any
            }
        }

        impl ToBytes for Any {
            fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
                self.0.to_bytes()
            }

            fn serialized_length(&self) -> usize {
                self.0.serialized_length()
            }
        }

        impl FromBytes for Any {
            fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
                let (inner, remainder) = String::from_bytes(bytes)?;
                Ok((Any(inner), remainder))
            }
        }

        let any = Any("Any test".to_string());
        round_trip(&any);
    }
}
