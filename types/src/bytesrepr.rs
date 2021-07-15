//! Contains serialization and deserialization code for types used throughout the system.
mod bytes;

// Can be removed once https://github.com/rust-lang/rustfmt/issues/3362 is resolved.
#[rustfmt::skip]
use alloc::vec;
use alloc::{
    alloc::{alloc, Layout},
    collections::{BTreeMap, BTreeSet, VecDeque},
    str,
    string::String,
    vec::Vec,
};
#[cfg(debug_assertions)]
use core::any;
use core::{marker::PhantomData, mem, ptr::NonNull};

use num_integer::Integer;
use num_rational::Ratio;
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use thiserror::Error;

pub use bytes::Bytes;

/// The number of bytes in a serialized `()`.
pub const UNIT_SERIALIZED_LENGTH: usize = 0;
/// The number of bytes in a serialized `bool`.
pub const BOOL_SERIALIZED_LENGTH: usize = 1;
/// The number of bytes in a serialized `i32`.
pub const I32_SERIALIZED_LENGTH: usize = mem::size_of::<i32>();
/// The number of bytes in a serialized `i64`.
pub const I64_SERIALIZED_LENGTH: usize = mem::size_of::<i64>();
/// The number of bytes in a serialized `u8`.
pub const U8_SERIALIZED_LENGTH: usize = mem::size_of::<u8>();
/// The number of bytes in a serialized `u16`.
pub const U16_SERIALIZED_LENGTH: usize = mem::size_of::<u16>();
/// The number of bytes in a serialized `u32`.
pub const U32_SERIALIZED_LENGTH: usize = mem::size_of::<u32>();
/// The number of bytes in a serialized `u64`.
pub const U64_SERIALIZED_LENGTH: usize = mem::size_of::<u64>();
/// The number of bytes in a serialized [`U128`](crate::U128).
pub const U128_SERIALIZED_LENGTH: usize = mem::size_of::<u128>();
/// The number of bytes in a serialized [`U256`](crate::U256).
pub const U256_SERIALIZED_LENGTH: usize = U128_SERIALIZED_LENGTH * 2;
/// The number of bytes in a serialized [`U512`](crate::U512).
pub const U512_SERIALIZED_LENGTH: usize = U256_SERIALIZED_LENGTH * 2;
/// The tag representing a `None` value.
pub const OPTION_NONE_TAG: u8 = 0;
/// The tag representing a `Some` value.
pub const OPTION_SOME_TAG: u8 = 1;
/// The tag representing an `Err` value.
pub const RESULT_ERR_TAG: u8 = 0;
/// The tag representing an `Ok` value.
pub const RESULT_OK_TAG: u8 = 1;

/// A type which can be serialized to a `Vec<u8>`.
pub trait ToBytes {
    /// Serializes `&self` to a `Vec<u8>`.
    fn to_bytes(&self) -> Result<Vec<u8>, Error>;
    /// Consumes `self` and serializes to a `Vec<u8>`.
    fn into_bytes(self) -> Result<Vec<u8>, Error>
    where
        Self: Sized,
    {
        self.to_bytes()
    }
    /// Returns the length of the `Vec<u8>` which would be returned from a successful call to
    /// `to_bytes()` or `into_bytes()`.  The data is not actually serialized, so this call is
    /// relatively cheap.
    fn serialized_length(&self) -> usize;
}

/// A type which can be deserialized from a `Vec<u8>`.
pub trait FromBytes: Sized {
    /// Deserializes the slice into `Self`.
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error>;
    /// Deserializes the `Vec<u8>` into `Self`.
    fn from_vec(bytes: Vec<u8>) -> Result<(Self, Vec<u8>), Error> {
        Self::from_bytes(bytes.as_slice()).map(|(x, remainder)| (x, Vec::from(remainder)))
    }
}

/// Returns a `Vec<u8>` initialized with sufficient capacity to hold `to_be_serialized` after
/// serialization.
pub fn unchecked_allocate_buffer<T: ToBytes>(to_be_serialized: &T) -> Vec<u8> {
    let serialized_length = to_be_serialized.serialized_length();
    Vec::with_capacity(serialized_length)
}

/// Returns a `Vec<u8>` initialized with sufficient capacity to hold `to_be_serialized` after
/// serialization, or an error if the capacity would exceed `u32::max_value()`.
pub fn allocate_buffer<T: ToBytes>(to_be_serialized: &T) -> Result<Vec<u8>, Error> {
    let serialized_length = to_be_serialized.serialized_length();
    if serialized_length > u32::max_value() as usize {
        return Err(Error::OutOfMemory);
    }
    Ok(Vec::with_capacity(serialized_length))
}

/// Serialization and deserialization errors.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "std", derive(Error))]
#[repr(u8)]
pub enum Error {
    /// Early end of stream while deserializing.
    #[cfg_attr(feature = "std", error("Deserialization error: early end of stream"))]
    EarlyEndOfStream = 0,
    /// Formatting error while deserializing.
    #[cfg_attr(feature = "std", error("Deserialization error: formatting"))]
    Formatting,
    /// Not all input bytes were consumed in [`deserialize`].
    #[cfg_attr(feature = "std", error("Deserialization error: left-over bytes"))]
    LeftOverBytes,
    /// Out of memory error.
    #[cfg_attr(feature = "std", error("Serialization error: out of memory"))]
    OutOfMemory,
}

/// Deserializes `bytes` into an instance of `T`.
///
/// Returns an error if the bytes cannot be deserialized into `T` or if not all of the input bytes
/// are consumed in the operation.
pub fn deserialize<T: FromBytes>(bytes: Vec<u8>) -> Result<T, Error> {
    let (t, remainder) = T::from_vec(bytes)?;
    if remainder.is_empty() {
        Ok(t)
    } else {
        Err(Error::LeftOverBytes)
    }
}

/// Serializes `t` into a `Vec<u8>`.
pub fn serialize(t: impl ToBytes) -> Result<Vec<u8>, Error> {
    t.into_bytes()
}

pub(crate) fn safe_split_at(bytes: &[u8], n: usize) -> Result<(&[u8], &[u8]), Error> {
    if n > bytes.len() {
        Err(Error::EarlyEndOfStream)
    } else {
        Ok(bytes.split_at(n))
    }
}

impl ToBytes for () {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(Vec::new())
    }

    fn serialized_length(&self) -> usize {
        UNIT_SERIALIZED_LENGTH
    }
}

impl FromBytes for () {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        Ok(((), bytes))
    }
}

impl ToBytes for bool {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        u8::from(*self).to_bytes()
    }

    fn serialized_length(&self) -> usize {
        BOOL_SERIALIZED_LENGTH
    }
}

impl FromBytes for bool {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        match bytes.split_first() {
            None => Err(Error::EarlyEndOfStream),
            Some((byte, rem)) => match byte {
                1 => Ok((true, rem)),
                0 => Ok((false, rem)),
                _ => Err(Error::Formatting),
            },
        }
    }
}

impl ToBytes for u8 {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(vec![*self])
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
    }
}

impl FromBytes for u8 {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        match bytes.split_first() {
            None => Err(Error::EarlyEndOfStream),
            Some((byte, rem)) => Ok((*byte, rem)),
        }
    }
}

impl ToBytes for i32 {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(self.to_le_bytes().to_vec())
    }

    fn serialized_length(&self) -> usize {
        I32_SERIALIZED_LENGTH
    }
}

impl FromBytes for i32 {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let mut result = [0u8; I32_SERIALIZED_LENGTH];
        let (bytes, remainder) = safe_split_at(bytes, I32_SERIALIZED_LENGTH)?;
        result.copy_from_slice(bytes);
        Ok((<i32>::from_le_bytes(result), remainder))
    }
}

impl ToBytes for i64 {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(self.to_le_bytes().to_vec())
    }

    fn serialized_length(&self) -> usize {
        I64_SERIALIZED_LENGTH
    }
}

impl FromBytes for i64 {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let mut result = [0u8; I64_SERIALIZED_LENGTH];
        let (bytes, remainder) = safe_split_at(bytes, I64_SERIALIZED_LENGTH)?;
        result.copy_from_slice(bytes);
        Ok((<i64>::from_le_bytes(result), remainder))
    }
}

impl ToBytes for u16 {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(self.to_le_bytes().to_vec())
    }

    fn serialized_length(&self) -> usize {
        U16_SERIALIZED_LENGTH
    }
}

impl FromBytes for u16 {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let mut result = [0u8; U16_SERIALIZED_LENGTH];
        let (bytes, remainder) = safe_split_at(bytes, U16_SERIALIZED_LENGTH)?;
        result.copy_from_slice(bytes);
        Ok((<u16>::from_le_bytes(result), remainder))
    }
}

impl ToBytes for u32 {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(self.to_le_bytes().to_vec())
    }

    fn serialized_length(&self) -> usize {
        U32_SERIALIZED_LENGTH
    }
}

impl FromBytes for u32 {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let mut result = [0u8; U32_SERIALIZED_LENGTH];
        let (bytes, remainder) = safe_split_at(bytes, U32_SERIALIZED_LENGTH)?;
        result.copy_from_slice(bytes);
        Ok((<u32>::from_le_bytes(result), remainder))
    }
}

impl ToBytes for u64 {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(self.to_le_bytes().to_vec())
    }

    fn serialized_length(&self) -> usize {
        U64_SERIALIZED_LENGTH
    }
}

impl FromBytes for u64 {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let mut result = [0u8; U64_SERIALIZED_LENGTH];
        let (bytes, remainder) = safe_split_at(bytes, U64_SERIALIZED_LENGTH)?;
        result.copy_from_slice(bytes);
        Ok((<u64>::from_le_bytes(result), remainder))
    }
}

impl ToBytes for String {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let bytes = self.as_bytes();
        u8_slice_to_bytes(bytes)
    }

    fn serialized_length(&self) -> usize {
        u8_slice_serialized_length(self.as_bytes())
    }
}

impl FromBytes for String {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (size, remainder) = u32::from_bytes(bytes)?;
        let (str_bytes, remainder) = safe_split_at(remainder, size as usize)?;
        let result = String::from_utf8(str_bytes.to_vec()).map_err(|_| Error::Formatting)?;
        Ok((result, remainder))
    }
}

fn ensure_efficient_serialization<T>() {
    #[cfg(debug_assertions)]
    debug_assert_ne!(
        any::type_name::<T>(),
        any::type_name::<u8>(),
        "You should use Bytes newtype wrapper for efficiency"
    );
}

fn iterator_serialized_length<'a, T: 'a + ToBytes>(ts: impl Iterator<Item = &'a T>) -> usize {
    U32_SERIALIZED_LENGTH + ts.map(ToBytes::serialized_length).sum::<usize>()
}

impl<T: ToBytes> ToBytes for Vec<T> {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        ensure_efficient_serialization::<T>();

        let mut result = try_vec_with_capacity(self.serialized_length())?;
        result.append(&mut (self.len() as u32).to_bytes()?);

        for item in self.iter() {
            result.append(&mut item.to_bytes()?);
        }

        Ok(result)
    }

    fn into_bytes(self) -> Result<Vec<u8>, Error> {
        ensure_efficient_serialization::<T>();

        let mut result = allocate_buffer(&self)?;
        result.append(&mut (self.len() as u32).to_bytes()?);

        for item in self {
            result.append(&mut item.into_bytes()?);
        }

        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        iterator_serialized_length(self.iter())
    }
}

// TODO Replace `try_vec_with_capacity` with `Vec::try_reserve_exact` once it's in stable.
fn try_vec_with_capacity<T>(capacity: usize) -> Result<Vec<T>, Error> {
    // see https://doc.rust-lang.org/src/alloc/raw_vec.rs.html#75-98
    let elem_size = mem::size_of::<T>();
    let alloc_size = capacity.checked_mul(elem_size).ok_or(Error::OutOfMemory)?;

    let ptr = if alloc_size == 0 {
        NonNull::<T>::dangling()
    } else {
        let align = mem::align_of::<T>();
        let layout = Layout::from_size_align(alloc_size, align).map_err(|_| Error::OutOfMemory)?;
        let raw_ptr = unsafe { alloc(layout) };
        let non_null_ptr = NonNull::<u8>::new(raw_ptr).ok_or(Error::OutOfMemory)?;
        non_null_ptr.cast()
    };
    unsafe { Ok(Vec::from_raw_parts(ptr.as_ptr(), 0, capacity)) }
}

fn vec_from_vec<T: FromBytes>(bytes: Vec<u8>) -> Result<(Vec<T>, Vec<u8>), Error> {
    ensure_efficient_serialization::<T>();

    Vec::<T>::from_bytes(bytes.as_slice()).map(|(x, remainder)| (x, Vec::from(remainder)))
}

impl<T: FromBytes> FromBytes for Vec<T> {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        ensure_efficient_serialization::<T>();

        let (count, mut stream) = u32::from_bytes(bytes)?;

        let mut result = try_vec_with_capacity(count as usize)?;
        for _ in 0..count {
            let (value, remainder) = T::from_bytes(stream)?;
            result.push(value);
            stream = remainder;
        }

        Ok((result, stream))
    }

    fn from_vec(bytes: Vec<u8>) -> Result<(Self, Vec<u8>), Error> {
        vec_from_vec(bytes)
    }
}

impl<T: ToBytes> ToBytes for VecDeque<T> {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let (slice1, slice2) = self.as_slices();
        let mut result = allocate_buffer(self)?;
        result.append(&mut (self.len() as u32).to_bytes()?);
        for item in slice1.iter().chain(slice2.iter()) {
            result.append(&mut item.to_bytes()?);
        }
        Ok(result)
    }

    fn into_bytes(self) -> Result<Vec<u8>, Error> {
        let vec: Vec<T> = self.into();
        vec.to_bytes()
    }

    fn serialized_length(&self) -> usize {
        let (slice1, slice2) = self.as_slices();
        iterator_serialized_length(slice1.iter().chain(slice2.iter()))
    }
}

impl<T: FromBytes> FromBytes for VecDeque<T> {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (vec, bytes) = Vec::from_bytes(bytes)?;
        Ok((VecDeque::from(vec), bytes))
    }

    fn from_vec(bytes: Vec<u8>) -> Result<(Self, Vec<u8>), Error> {
        let (vec, bytes) = vec_from_vec(bytes)?;
        Ok((VecDeque::from(vec), bytes))
    }
}

macro_rules! impl_to_from_bytes_for_array {
    ($($N:literal)+) => {
        $(
            impl ToBytes for [u8; $N] {
                #[inline(always)]
                fn to_bytes(&self) -> Result<Vec<u8>, Error> {
                    Ok(self.to_vec())
                }

                #[inline(always)]
                fn serialized_length(&self) -> usize { $N }
            }

            impl FromBytes for [u8; $N] {
                fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
                    let (bytes, rem) = safe_split_at(bytes, $N)?;
                    // SAFETY: safe_split_at makes sure `bytes` is exactly $N bytes.
                    let ptr = bytes.as_ptr() as *const [u8; $N];
                    let result = unsafe { *ptr };
                    Ok((result, rem))
                }
            }
        )+
    }
}

impl_to_from_bytes_for_array! {
     0  1  2  3  4  5  6  7  8  9
    10 11 12 13 14 15 16 17 18 19
    20 21 22 23 24 25 26 27 28 29
    30 31 32
    33
    64 128 256 512
}

impl<V: ToBytes> ToBytes for BTreeSet<V> {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;

        let num_keys = self.len() as u32;
        result.append(&mut num_keys.to_bytes()?);

        for value in self.iter() {
            result.append(&mut value.to_bytes()?);
        }

        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        U32_SERIALIZED_LENGTH + self.iter().map(|v| v.serialized_length()).sum::<usize>()
    }
}

impl<V: FromBytes + Ord> FromBytes for BTreeSet<V> {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (num_keys, mut stream) = u32::from_bytes(bytes)?;
        let mut result = BTreeSet::new();
        for _ in 0..num_keys {
            let (v, rem) = V::from_bytes(stream)?;
            result.insert(v);
            stream = rem;
        }
        Ok((result, stream))
    }
}

impl<K, V> ToBytes for BTreeMap<K, V>
where
    K: ToBytes,
    V: ToBytes,
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;

        let num_keys = self.len() as u32;
        result.append(&mut num_keys.to_bytes()?);

        for (key, value) in self.iter() {
            result.append(&mut key.to_bytes()?);
            result.append(&mut value.to_bytes()?);
        }

        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        U32_SERIALIZED_LENGTH
            + self
                .iter()
                .map(|(key, value)| key.serialized_length() + value.serialized_length())
                .sum::<usize>()
    }
}

impl<K, V> FromBytes for BTreeMap<K, V>
where
    K: FromBytes + Ord,
    V: FromBytes,
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (num_keys, mut stream) = u32::from_bytes(bytes)?;
        let mut result = BTreeMap::new();
        for _ in 0..num_keys {
            let (k, rem) = K::from_bytes(stream)?;
            let (v, rem) = V::from_bytes(rem)?;
            result.insert(k, v);
            stream = rem;
        }
        Ok((result, stream))
    }
}

impl<T: ToBytes> ToBytes for Option<T> {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        match self {
            None => Ok(vec![OPTION_NONE_TAG]),
            Some(v) => {
                let mut result = allocate_buffer(self)?;
                result.push(OPTION_SOME_TAG);

                let mut value = v.to_bytes()?;
                result.append(&mut value);

                Ok(result)
            }
        }
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
            + match self {
                Some(v) => v.serialized_length(),
                None => 0,
            }
    }
}

impl<T: FromBytes> FromBytes for Option<T> {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (tag, rem) = u8::from_bytes(bytes)?;
        match tag {
            OPTION_NONE_TAG => Ok((None, rem)),
            OPTION_SOME_TAG => {
                let (t, rem) = T::from_bytes(rem)?;
                Ok((Some(t), rem))
            }
            _ => Err(Error::Formatting),
        }
    }
}

impl<T: ToBytes, E: ToBytes> ToBytes for Result<T, E> {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        let (variant, mut value) = match self {
            Err(error) => (RESULT_ERR_TAG, error.to_bytes()?),
            Ok(result) => (RESULT_OK_TAG, result.to_bytes()?),
        };
        result.push(variant);
        result.append(&mut value);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
            + match self {
                Ok(ok) => ok.serialized_length(),
                Err(error) => error.serialized_length(),
            }
    }
}

impl<T: FromBytes, E: FromBytes> FromBytes for Result<T, E> {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (variant, rem) = u8::from_bytes(bytes)?;
        match variant {
            RESULT_ERR_TAG => {
                let (value, rem) = E::from_bytes(rem)?;
                Ok((Err(value), rem))
            }
            RESULT_OK_TAG => {
                let (value, rem) = T::from_bytes(rem)?;
                Ok((Ok(value), rem))
            }
            _ => Err(Error::Formatting),
        }
    }
}

impl<T1: ToBytes> ToBytes for (T1,) {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.0.to_bytes()
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
    }
}

impl<T1: FromBytes> FromBytes for (T1,) {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        Ok(((t1,), remainder))
    }
}

impl<T1: ToBytes, T2: ToBytes> ToBytes for (T1, T2) {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length() + self.1.serialized_length()
    }
}

impl<T1: FromBytes, T2: FromBytes> FromBytes for (T1, T2) {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        Ok(((t1, t2), remainder))
    }
}

impl<T1: ToBytes, T2: ToBytes, T3: ToBytes> ToBytes for (T1, T2, T3) {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length() + self.1.serialized_length() + self.2.serialized_length()
    }
}

impl<T1: FromBytes, T2: FromBytes, T3: FromBytes> FromBytes for (T1, T2, T3) {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        Ok(((t1, t2, t3), remainder))
    }
}

impl<T1: ToBytes, T2: ToBytes, T3: ToBytes, T4: ToBytes> ToBytes for (T1, T2, T3, T4) {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
    }
}

impl<T1: FromBytes, T2: FromBytes, T3: FromBytes, T4: FromBytes> FromBytes for (T1, T2, T3, T4) {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4), remainder))
    }
}

impl<T1: ToBytes, T2: ToBytes, T3: ToBytes, T4: ToBytes, T5: ToBytes> ToBytes
    for (T1, T2, T3, T4, T5)
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        result.append(&mut self.4.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
            + self.4.serialized_length()
    }
}

impl<T1: FromBytes, T2: FromBytes, T3: FromBytes, T4: FromBytes, T5: FromBytes> FromBytes
    for (T1, T2, T3, T4, T5)
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        let (t5, remainder) = T5::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4, t5), remainder))
    }
}

impl<T1: ToBytes, T2: ToBytes, T3: ToBytes, T4: ToBytes, T5: ToBytes, T6: ToBytes> ToBytes
    for (T1, T2, T3, T4, T5, T6)
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        result.append(&mut self.4.to_bytes()?);
        result.append(&mut self.5.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
            + self.4.serialized_length()
            + self.5.serialized_length()
    }
}

impl<T1: FromBytes, T2: FromBytes, T3: FromBytes, T4: FromBytes, T5: FromBytes, T6: FromBytes>
    FromBytes for (T1, T2, T3, T4, T5, T6)
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        let (t5, remainder) = T5::from_bytes(remainder)?;
        let (t6, remainder) = T6::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4, t5, t6), remainder))
    }
}

impl<T1: ToBytes, T2: ToBytes, T3: ToBytes, T4: ToBytes, T5: ToBytes, T6: ToBytes, T7: ToBytes>
    ToBytes for (T1, T2, T3, T4, T5, T6, T7)
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        result.append(&mut self.4.to_bytes()?);
        result.append(&mut self.5.to_bytes()?);
        result.append(&mut self.6.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
            + self.4.serialized_length()
            + self.5.serialized_length()
            + self.6.serialized_length()
    }
}

impl<
        T1: FromBytes,
        T2: FromBytes,
        T3: FromBytes,
        T4: FromBytes,
        T5: FromBytes,
        T6: FromBytes,
        T7: FromBytes,
    > FromBytes for (T1, T2, T3, T4, T5, T6, T7)
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        let (t5, remainder) = T5::from_bytes(remainder)?;
        let (t6, remainder) = T6::from_bytes(remainder)?;
        let (t7, remainder) = T7::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4, t5, t6, t7), remainder))
    }
}

impl<
        T1: ToBytes,
        T2: ToBytes,
        T3: ToBytes,
        T4: ToBytes,
        T5: ToBytes,
        T6: ToBytes,
        T7: ToBytes,
        T8: ToBytes,
    > ToBytes for (T1, T2, T3, T4, T5, T6, T7, T8)
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        result.append(&mut self.4.to_bytes()?);
        result.append(&mut self.5.to_bytes()?);
        result.append(&mut self.6.to_bytes()?);
        result.append(&mut self.7.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
            + self.4.serialized_length()
            + self.5.serialized_length()
            + self.6.serialized_length()
            + self.7.serialized_length()
    }
}

impl<
        T1: FromBytes,
        T2: FromBytes,
        T3: FromBytes,
        T4: FromBytes,
        T5: FromBytes,
        T6: FromBytes,
        T7: FromBytes,
        T8: FromBytes,
    > FromBytes for (T1, T2, T3, T4, T5, T6, T7, T8)
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        let (t5, remainder) = T5::from_bytes(remainder)?;
        let (t6, remainder) = T6::from_bytes(remainder)?;
        let (t7, remainder) = T7::from_bytes(remainder)?;
        let (t8, remainder) = T8::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4, t5, t6, t7, t8), remainder))
    }
}

impl<
        T1: ToBytes,
        T2: ToBytes,
        T3: ToBytes,
        T4: ToBytes,
        T5: ToBytes,
        T6: ToBytes,
        T7: ToBytes,
        T8: ToBytes,
        T9: ToBytes,
    > ToBytes for (T1, T2, T3, T4, T5, T6, T7, T8, T9)
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        result.append(&mut self.4.to_bytes()?);
        result.append(&mut self.5.to_bytes()?);
        result.append(&mut self.6.to_bytes()?);
        result.append(&mut self.7.to_bytes()?);
        result.append(&mut self.8.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
            + self.4.serialized_length()
            + self.5.serialized_length()
            + self.6.serialized_length()
            + self.7.serialized_length()
            + self.8.serialized_length()
    }
}

impl<
        T1: FromBytes,
        T2: FromBytes,
        T3: FromBytes,
        T4: FromBytes,
        T5: FromBytes,
        T6: FromBytes,
        T7: FromBytes,
        T8: FromBytes,
        T9: FromBytes,
    > FromBytes for (T1, T2, T3, T4, T5, T6, T7, T8, T9)
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        let (t5, remainder) = T5::from_bytes(remainder)?;
        let (t6, remainder) = T6::from_bytes(remainder)?;
        let (t7, remainder) = T7::from_bytes(remainder)?;
        let (t8, remainder) = T8::from_bytes(remainder)?;
        let (t9, remainder) = T9::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4, t5, t6, t7, t8, t9), remainder))
    }
}

impl<
        T1: ToBytes,
        T2: ToBytes,
        T3: ToBytes,
        T4: ToBytes,
        T5: ToBytes,
        T6: ToBytes,
        T7: ToBytes,
        T8: ToBytes,
        T9: ToBytes,
        T10: ToBytes,
    > ToBytes for (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10)
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = allocate_buffer(self)?;
        result.append(&mut self.0.to_bytes()?);
        result.append(&mut self.1.to_bytes()?);
        result.append(&mut self.2.to_bytes()?);
        result.append(&mut self.3.to_bytes()?);
        result.append(&mut self.4.to_bytes()?);
        result.append(&mut self.5.to_bytes()?);
        result.append(&mut self.6.to_bytes()?);
        result.append(&mut self.7.to_bytes()?);
        result.append(&mut self.8.to_bytes()?);
        result.append(&mut self.9.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
            + self.1.serialized_length()
            + self.2.serialized_length()
            + self.3.serialized_length()
            + self.4.serialized_length()
            + self.5.serialized_length()
            + self.6.serialized_length()
            + self.7.serialized_length()
            + self.8.serialized_length()
            + self.9.serialized_length()
    }
}

impl<
        T1: FromBytes,
        T2: FromBytes,
        T3: FromBytes,
        T4: FromBytes,
        T5: FromBytes,
        T6: FromBytes,
        T7: FromBytes,
        T8: FromBytes,
        T9: FromBytes,
        T10: FromBytes,
    > FromBytes for (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10)
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (t1, remainder) = T1::from_bytes(bytes)?;
        let (t2, remainder) = T2::from_bytes(remainder)?;
        let (t3, remainder) = T3::from_bytes(remainder)?;
        let (t4, remainder) = T4::from_bytes(remainder)?;
        let (t5, remainder) = T5::from_bytes(remainder)?;
        let (t6, remainder) = T6::from_bytes(remainder)?;
        let (t7, remainder) = T7::from_bytes(remainder)?;
        let (t8, remainder) = T8::from_bytes(remainder)?;
        let (t9, remainder) = T9::from_bytes(remainder)?;
        let (t10, remainder) = T10::from_bytes(remainder)?;
        Ok(((t1, t2, t3, t4, t5, t6, t7, t8, t9, t10), remainder))
    }
}

impl ToBytes for str {
    #[inline]
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        u8_slice_to_bytes(self.as_bytes())
    }

    #[inline]
    fn serialized_length(&self) -> usize {
        u8_slice_serialized_length(self.as_bytes())
    }
}

impl ToBytes for &str {
    #[inline(always)]
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        (*self).to_bytes()
    }

    #[inline(always)]
    fn serialized_length(&self) -> usize {
        (*self).serialized_length()
    }
}

impl<T> ToBytes for Ratio<T>
where
    T: Clone + Integer + ToBytes,
{
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        if self.denom().is_zero() {
            return Err(Error::Formatting);
        }
        (self.numer().clone(), self.denom().clone()).into_bytes()
    }

    fn serialized_length(&self) -> usize {
        (self.numer().clone(), self.denom().clone()).serialized_length()
    }
}

impl<T> FromBytes for Ratio<T>
where
    T: Clone + FromBytes + Integer,
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let ((numer, denom), rem): ((T, T), &[u8]) = FromBytes::from_bytes(bytes)?;
        if denom.is_zero() {
            return Err(Error::Formatting);
        }
        Ok((Ratio::new(numer, denom), rem))
    }
}

/// Serializes a slice of bytes with a length prefix.
///
/// This function is serializing a slice of bytes with an addition of a 4 byte length prefix.
///
/// For safety you should prefer to use [`vec_u8_to_bytes`]. For efficiency reasons you should also
/// avoid using serializing Vec<u8>.
fn u8_slice_to_bytes(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let serialized_length = u8_slice_serialized_length(bytes);
    let mut vec = try_vec_with_capacity(serialized_length)?;
    let length_prefix = bytes.len() as u32;
    let length_prefix_bytes = length_prefix.to_le_bytes();
    vec.extend_from_slice(&length_prefix_bytes);
    vec.extend_from_slice(bytes);
    Ok(vec)
}

/// Serializes a vector of bytes with a length prefix.
///
/// For efficiency you should avoid serializing Vec<u8>.
#[allow(clippy::ptr_arg)]
#[inline]
pub(crate) fn vec_u8_to_bytes(vec: &Vec<u8>) -> Result<Vec<u8>, Error> {
    u8_slice_to_bytes(vec.as_slice())
}

/// Returns serialized length of serialized slice of bytes.
///
/// This function adds a length prefix in the beginning.
#[inline(always)]
fn u8_slice_serialized_length(bytes: &[u8]) -> usize {
    U32_SERIALIZED_LENGTH + bytes.len()
}

#[allow(clippy::ptr_arg)]
#[inline]
pub(crate) fn vec_u8_serialized_length(vec: &Vec<u8>) -> usize {
    u8_slice_serialized_length(vec.as_slice())
}

// This test helper is not intended to be used by third party crates.
#[doc(hidden)]
/// Returns `true` if a we can serialize and then deserialize a value
pub fn test_serialization_roundtrip<T>(t: &T)
where
    T: alloc::fmt::Debug + ToBytes + FromBytes + PartialEq,
{
    let serialized = ToBytes::to_bytes(t).expect("Unable to serialize data");
    assert_eq!(
        serialized.len(),
        t.serialized_length(),
        "\nLength of serialized data: {},\nserialized_length() yielded: {},\nserialized data: {:?}, t is {:?}",
        serialized.len(),
        t.serialized_length(),
        serialized,
        t
    );
    let deserialized = deserialize::<T>(serialized).expect("Unable to deserialize data");
    assert!(*t == deserialized)
}

/// A convenient writer object that deals with serializing structs. Binary representation is very
/// similar to a serialized [`BTreeMap<u64, Vec<u8>>`], or a vector of 2-element tuple, but with
/// extra type safety.
pub struct StructWriter<K> {
    buf: Vec<u8>,
    items: u32,
    marker: PhantomData<K>,
}

impl<K> Default for StructWriter<K> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            items: 0,
            marker: PhantomData,
        }
    }
}

impl<K: ToPrimitive> StructWriter<K> {
    /// Creates new writer for a struct
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes a pair of a key, and a value
    pub fn write_pair<V>(&mut self, key: K, value: V) -> Result<(), Error>
    where
        V: ToBytes,
    {
        let key_serializable = key.to_u64().unwrap();
        let value_serializable = value.to_bytes()?;
        // Serializes a key as a number with fixed length bytes
        self.buf.append(&mut key_serializable.to_bytes()?);
        // Serializes a value as a length-prefixed byte array so deserializer can skip unknown
        // fields while iterating.
        self.buf
            .append(&mut Bytes::from(value_serializable).to_bytes()?);
        self.items += 1;
        Ok(())
    }

    /// Returns serialized structure
    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        let mut length_prefix = self.items.to_bytes()?;

        let mut result = Vec::with_capacity(length_prefix.len() + self.buf.len());
        result.append(&mut length_prefix);
        result.append(&mut self.buf);

        Ok(result)
    }
}

/// Calculates size of a serialized struct with given fields sizes
pub fn serialized_struct_fields_length(fields: &[usize]) -> usize {
    let attributes: usize = fields
        .iter()
        // key serialied length+ u32 length prefix + length of serialized field
        .map(|&field_length| U64_SERIALIZED_LENGTH + U32_SERIALIZED_LENGTH + field_length)
        .sum();
    U32_SERIALIZED_LENGTH + attributes
}

/// Reader of a serialized structs with [`StructWriter`]
/// NOTE: This reader implementation does not try to be re-entrant. It means that once
/// [`StructReader::read_key`] or [`StructReader::read_value`] errors, then you should not attempt
/// to read from the same reader again.
pub struct StructReader<'a> {
    buf: &'a [u8],
    length_prefix: Option<u32>,
}

impl<'a> StructReader<'a> {
    /// Creates new reader with given byte stream
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            length_prefix: None,
        }
    }

    /// Reads a length prefix if not read already
    fn read_length_prefix(&mut self) -> Result<u32, Error> {
        match self.length_prefix {
            Some(value) => Ok(value),
            None => {
                let (length_prefix, rem) = FromBytes::from_bytes(self.buf)?;
                self.buf = rem;
                self.length_prefix = Some(length_prefix);
                Ok(length_prefix)
            }
        }
    }

    /// Reads next key in a struct
    pub fn read_key(&mut self) -> Result<Option<u64>, Error> {
        let length_prefix = self.read_length_prefix()?;
        if length_prefix == 0 {
            return Ok(None);
        }

        let (key_value, rem): (u64, _) = FromBytes::from_bytes(self.buf)?;
        self.buf = rem;
        Ok(Some(key_value))
    }

    /// Reads next value from a struct
    pub fn read_value<V>(&mut self) -> Result<V, Error>
    where
        V: FromBytes,
    {
        let value_bytes = self.read_value_raw()?;
        // Deserialize the data contained
        let v: V = deserialize(value_bytes)?;
        Ok(v)
    }

    /// Reads next element by trying to deserialize it, and moves the internal buffer forward once
    /// it succeed
    fn advance<T>(&mut self) -> Result<T, Error>
    where
        T: FromBytes,
    {
        let (value, rem) = FromBytes::from_bytes(self.buf)?;
        self.buf = rem;
        Ok(value)
    }

    fn read_value_raw(&mut self) -> Result<Vec<u8>, Error> {
        let length_prefix = self.length_prefix.ok_or(Error::Formatting)?;
        if length_prefix == 0 {
            return Err(Error::Formatting);
        }
        let value_bytes: Bytes = self.advance()?;
        self.length_prefix = Some(length_prefix - 1);
        Ok(value_bytes.into())
    }

    /// Skips next value. Useful in case where the key is unknown, or is deprecated/removed.
    pub fn skip_value(&mut self) -> Result<(), Error> {
        let _bytes = self.read_value_raw()?;
        Ok(())
    }

    /// Returns remaining buffer
    pub fn finish(self) -> &'a [u8] {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::U512;
    use alloc::{str::FromStr, string::ToString};
    use num_derive::{FromPrimitive, ToPrimitive};
    use num_traits::FromPrimitive;

    #[test]
    fn should_not_serialize_zero_denominator() {
        let malicious = Ratio::new_raw(1, 0);
        assert_eq!(malicious.to_bytes().unwrap_err(), Error::Formatting);
    }

    #[test]
    fn should_not_deserialize_zero_denominator() {
        let malicious_bytes = (1u64, 0u64).to_bytes().unwrap();
        let result: Result<Ratio<u64>, Error> = super::deserialize(malicious_bytes);
        assert_eq!(result.unwrap_err(), Error::Formatting);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "You should use Bytes newtype wrapper for efficiency")]
    fn should_fail_to_serialize_slice_of_u8() {
        let bytes = b"0123456789".to_vec();
        bytes.to_bytes().unwrap();
    }

    #[derive(ToPrimitive, FromPrimitive, Eq, PartialEq, Ord, PartialOrd)]
    enum Key {
        Attr1 = 100,
        Attr2,
        Attr3,
        Attr4,
        Attr5,
    }

    struct FooV1 {
        attr1: u64,
        attr2: Vec<String>,
        attr3: String,
        attr4: u64,
    }

    impl ToBytes for FooV1 {
        fn to_bytes(&self) -> Result<Vec<u8>, Error> {
            let mut writer = StructWriter::new();
            writer.write_pair(Key::Attr1, self.attr1)?;
            writer.write_pair(Key::Attr2, self.attr2.clone())?;
            writer.write_pair(Key::Attr3, self.attr3.clone())?;
            writer.write_pair(Key::Attr4, self.attr4)?;
            writer.finish()
        }

        fn serialized_length(&self) -> usize {
            todo!()
        }
    }

    struct FooV2 {
        attr1: u64,
        attr2: Vec<u32>, // was Vec<String>
        attr3: String,
        attr4: String,
        new_field: U512,
        unknown_fields: BTreeMap<u64, Vec<u8>>,
    }

    impl Default for FooV2 {
        fn default() -> Self {
            FooV2 {
                attr1: 0,
                attr2: Vec::new(),
                attr3: String::new(),
                attr4: String::new(),
                new_field: U512::MAX,
                unknown_fields: BTreeMap::new(),
            }
        }
    }

    impl ToBytes for FooV2 {
        fn to_bytes(&self) -> Result<Vec<u8>, Error> {
            let mut writer = StructWriter::new();
            writer.write_pair(Key::Attr1, self.attr1)?;
            writer.write_pair(Key::Attr2, self.attr2.clone())?;
            writer.write_pair(Key::Attr3, self.attr3.clone())?;
            writer.write_pair(Key::Attr4, self.attr4.clone())?;
            writer.finish()
        }

        fn serialized_length(&self) -> usize {
            todo!()
        }
    }

    impl FromBytes for FooV2 {
        fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
            let mut reader = StructReader::new(bytes);

            let mut obj = FooV2::default();

            while let Some(key) = reader.read_key()? {
                match Key::from_u64(key) {
                    Some(Key::Attr1) => {
                        obj.attr1 = reader.read_value()?;
                    }
                    Some(Key::Attr2) => {
                        let old_attr2: Vec<String> = reader.read_value()?;
                        obj.attr2 = old_attr2
                            .into_iter()
                            .filter_map(|value| u32::from_str(&value).ok())
                            .collect();
                    }
                    Some(Key::Attr3) => {
                        obj.attr3 = reader.read_value()?;
                    }
                    Some(Key::Attr4) => {
                        let old_attr4: u64 = reader.read_value()?;
                        obj.attr4 = old_attr4.to_string();
                    }
                    None | Some(_) => {
                        obj.unknown_fields.insert(key, reader.read_value_raw()?);
                    }
                }
            }

            Ok((obj, reader.finish()))
        }
    }

    #[test]
    fn struct_serializer() {
        let foo_v1 = FooV1 {
            attr1: 123456789u64,
            attr2: vec!["50".to_string(), "100".to_string(), "150".to_string()],
            attr3: "Hello, world!".to_string(),
            attr4: 987654321u64,
        };

        let foo_v1_bytes = foo_v1.to_bytes().unwrap();

        let foo_v2: FooV2 = deserialize(foo_v1_bytes).unwrap();

        assert_eq!(foo_v1.attr1, foo_v2.attr1);
        assert_eq!(
            foo_v1.attr2,
            foo_v2
                .attr2
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<String>>()
        );
        assert_eq!(foo_v1.attr3, foo_v2.attr3);
        assert_eq!(foo_v1.attr4.to_string(), foo_v2.attr4);
        assert_eq!(foo_v2.new_field, U512::MAX);
    }

    #[test]
    fn should_not_panic_when_used_incorrectly() {
        // 1 element in struct, first elements' key is 0, there should be a value but there is none
        let buffer_1 = (0u32, 0u64).to_bytes().unwrap();
        let mut reader_1 = StructReader::new(&buffer_1);
        assert_eq!(reader_1.read_value::<()>().unwrap_err(), Error::Formatting);
    }
}

#[cfg(test)]
mod proptests {
    use std::collections::VecDeque;

    use proptest::{collection::vec, prelude::*};

    use crate::{
        bytesrepr::{self, bytes::gens::bytes_arb, ToBytes},
        gens::*,
    };

    proptest! {
        #[test]
        fn test_bool(u in any::<bool>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u8(u in any::<u8>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u16(u in any::<u16>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u32(u in any::<u32>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_i32(u in any::<i32>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u64(u in any::<u64>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_i64(u in any::<i64>()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u8_slice_32(s in u8_slice_32()) {
            bytesrepr::test_serialization_roundtrip(&s);
        }

        #[test]
        fn test_vec_u8(u in bytes_arb(1..100)) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_vec_i32(u in vec(any::<i32>(), 1..100)) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_vecdeque_i32((front, back) in (vec(any::<i32>(), 1..100), vec(any::<i32>(), 1..100))) {
            let mut vec_deque = VecDeque::new();
            for f in front {
                vec_deque.push_front(f);
            }
            for f in back {
                vec_deque.push_back(f);
            }
            bytesrepr::test_serialization_roundtrip(&vec_deque);
        }

        #[test]
        fn test_vec_vec_u8(u in vec(bytes_arb(1..100), 10)) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_uref_map(m in named_keys_arb(20)) {
            bytesrepr::test_serialization_roundtrip(&m);
        }

        #[test]
        fn test_array_u8_32(arr in any::<[u8; 32]>()) {
            bytesrepr::test_serialization_roundtrip(&arr);
        }

        #[test]
        fn test_string(s in "\\PC*") {
            bytesrepr::test_serialization_roundtrip(&s);
        }

        #[test]
        fn test_str(s in "\\PC*") {
            let not_a_string_object = s.as_str();
            not_a_string_object.to_bytes().expect("should serialize a str");
        }

        #[test]
        fn test_option(o in proptest::option::of(key_arb())) {
            bytesrepr::test_serialization_roundtrip(&o);
        }

        #[test]
        fn test_unit(unit in Just(())) {
            bytesrepr::test_serialization_roundtrip(&unit);
        }

        #[test]
        fn test_u128_serialization(u in u128_arb()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u256_serialization(u in u256_arb()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_u512_serialization(u in u512_arb()) {
            bytesrepr::test_serialization_roundtrip(&u);
        }

        #[test]
        fn test_key_serialization(key in key_arb()) {
            bytesrepr::test_serialization_roundtrip(&key);
        }

        #[test]
        fn test_cl_value_serialization(cl_value in cl_value_arb()) {
            bytesrepr::test_serialization_roundtrip(&cl_value);
        }

        #[test]
        fn test_access_rights(access_right in access_rights_arb()) {
            bytesrepr::test_serialization_roundtrip(&access_right);
        }

        #[test]
        fn test_uref(uref in uref_arb()) {
            bytesrepr::test_serialization_roundtrip(&uref);
        }

        #[test]
        fn test_account_hash(pk in account_hash_arb()) {
            bytesrepr::test_serialization_roundtrip(&pk);
        }

        #[test]
        fn test_result(result in result_arb()) {
            bytesrepr::test_serialization_roundtrip(&result);
        }

        #[test]
        fn test_phase_serialization(phase in phase_arb()) {
            bytesrepr::test_serialization_roundtrip(&phase);
        }

        #[test]
        fn test_protocol_version(protocol_version in protocol_version_arb()) {
            bytesrepr::test_serialization_roundtrip(&protocol_version);
        }

        #[test]
        fn test_sem_ver(sem_ver in sem_ver_arb()) {
            bytesrepr::test_serialization_roundtrip(&sem_ver);
        }

        #[test]
        fn test_tuple1(t in (any::<u8>(),)) {
            bytesrepr::test_serialization_roundtrip(&t);
        }

        #[test]
        fn test_tuple2(t in (any::<u8>(),any::<u32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }

        #[test]
        fn test_tuple3(t in (any::<u8>(),any::<u32>(),any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }

        #[test]
        fn test_tuple4(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_tuple5(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_tuple6(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_tuple7(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_tuple8(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_tuple9(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_tuple10(t in (any::<u8>(),any::<u32>(),any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
        #[test]
        fn test_ratio_u64(t in (any::<u64>(), 1..u64::max_value())) {
            bytesrepr::test_serialization_roundtrip(&t);
        }
    }
}
