use alloc::{
    string::String,
    vec::{IntoIter, Vec},
};
use core::{
    cmp, fmt,
    iter::FromIterator,
    mem,
    ops::{Deref, Index, Range, RangeFrom, RangeFull, RangeTo},
    slice,
};

use datasize::DataSize;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use serde::{
    de::{Error as SerdeError, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

use super::{Error, FromBytes, ToBytes};
use crate::{check_summed_hex, CLType, CLTyped};

/// A newtype wrapper for bytes that has efficient serialization routines.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Default, Hash)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    /// Constructs a new, empty vector of bytes.
    pub fn new() -> Bytes {
        Bytes::default()
    }

    /// Returns reference to inner container.
    #[inline]
    pub fn inner_bytes(&self) -> &Vec<u8> {
        &self.0
    }

    /// Extracts a slice containing the entire vector.
    pub fn as_slice(&self) -> &[u8] {
        self
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(vec: Vec<u8>) -> Self {
        Self(vec)
    }
}

impl From<Bytes> for Vec<u8> {
    fn from(bytes: Bytes) -> Self {
        bytes.0
    }
}

impl From<&[u8]> for Bytes {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }
}

impl CLTyped for Bytes {
    fn cl_type() -> CLType {
        <Vec<u8>>::cl_type()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl ToBytes for Bytes {
    #[inline(always)]
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        super::vec_u8_to_bytes(&self.0)
    }

    #[inline(always)]
    fn into_bytes(self) -> Result<Vec<u8>, Error> {
        super::vec_u8_to_bytes(&self.0)
    }

    #[inline(always)]
    fn serialized_length(&self) -> usize {
        super::vec_u8_serialized_length(&self.0)
    }
}

impl FromBytes for Bytes {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), super::Error> {
        let (size, remainder) = u32::from_bytes(bytes)?;
        let (result, remainder) = super::safe_split_at(remainder, size as usize)?;
        Ok((Bytes(result.to_vec()), remainder))
    }

    fn from_vec(stream: Vec<u8>) -> Result<(Self, Vec<u8>), Error> {
        let (size, mut stream) = u32::from_vec(stream)?;

        if size as usize > stream.len() {
            Err(Error::EarlyEndOfStream)
        } else {
            let remainder = stream.split_off(size as usize);
            Ok((Bytes(stream), remainder))
        }
    }
}

impl Index<usize> for Bytes {
    type Output = u8;

    fn index(&self, index: usize) -> &u8 {
        let Bytes(ref dat) = self;
        &dat[index]
    }
}

impl Index<Range<usize>> for Bytes {
    type Output = [u8];

    fn index(&self, index: Range<usize>) -> &[u8] {
        let &Bytes(ref dat) = self;
        &dat[index]
    }
}

impl Index<RangeTo<usize>> for Bytes {
    type Output = [u8];

    fn index(&self, index: RangeTo<usize>) -> &[u8] {
        let &Bytes(ref dat) = self;
        &dat[index]
    }
}

impl Index<RangeFrom<usize>> for Bytes {
    type Output = [u8];

    fn index(&self, index: RangeFrom<usize>) -> &[u8] {
        let &Bytes(ref dat) = self;
        &dat[index]
    }
}

impl Index<RangeFull> for Bytes {
    type Output = [u8];

    fn index(&self, _: RangeFull) -> &[u8] {
        let &Bytes(ref dat) = self;
        &dat[..]
    }
}

impl FromIterator<u8> for Bytes {
    #[inline]
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Bytes {
        let vec = Vec::from_iter(iter);
        Bytes(vec)
    }
}

impl<'a> IntoIterator for &'a Bytes {
    type Item = &'a u8;

    type IntoIter = slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for Bytes {
    type Item = u8;

    type IntoIter = IntoIter<u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl DataSize for Bytes {
    const IS_DYNAMIC: bool = true;

    const STATIC_HEAP_SIZE: usize = 0;

    fn estimate_heap_size(&self) -> usize {
        self.0.capacity() * mem::size_of::<u8>()
    }
}

const RANDOM_BYTES_MAX_LENGTH: usize = 100;

impl Distribution<Bytes> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Bytes {
        let len = rng.gen_range(0..RANDOM_BYTES_MAX_LENGTH);
        let mut result = Vec::with_capacity(len);
        for _ in 0..len {
            result.push(rng.gen());
        }
        result.into()
    }
}

struct BytesVisitor;

impl<'de> Visitor<'de> for BytesVisitor {
    type Value = Bytes;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("byte array")
    }

    fn visit_seq<V>(self, mut visitor: V) -> Result<Bytes, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let len = cmp::min(visitor.size_hint().unwrap_or(0), 4096);
        let mut bytes = Vec::with_capacity(len);

        while let Some(b) = visitor.next_element()? {
            bytes.push(b);
        }

        Ok(Bytes::from(bytes))
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Bytes, E>
    where
        E: SerdeError,
    {
        Ok(Bytes::from(v))
    }

    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Bytes, E>
    where
        E: SerdeError,
    {
        Ok(Bytes::from(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Bytes, E>
    where
        E: SerdeError,
    {
        Ok(Bytes::from(v.as_bytes()))
    }

    fn visit_string<E>(self, v: String) -> Result<Bytes, E>
    where
        E: SerdeError,
    {
        Ok(Bytes::from(v.into_bytes()))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let hex_string = String::deserialize(deserializer)?;
            check_summed_hex::decode(&hex_string)
                .map(Bytes)
                .map_err(SerdeError::custom)
        } else {
            let bytes = deserializer.deserialize_byte_buf(BytesVisitor)?;
            Ok(bytes)
        }
    }
}

impl Serialize for Bytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            check_summed_hex::encode(&self.0).serialize(serializer)
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bytesrepr::{self, Error, FromBytes, ToBytes, U32_SERIALIZED_LENGTH};
    use alloc::vec::Vec;

    use serde_json::json;
    use serde_test::{assert_tokens, Configure, Token};

    use super::Bytes;

    const TRUTH: &[u8] = &[0xde, 0xad, 0xbe, 0xef];

    #[test]
    fn vec_u8_from_bytes() {
        let data: Bytes = vec![1, 2, 3, 4, 5].into();
        let data_bytes = data.to_bytes().unwrap();
        assert!(Bytes::from_bytes(&data_bytes[..U32_SERIALIZED_LENGTH / 2]).is_err());
        assert!(Bytes::from_bytes(&data_bytes[..U32_SERIALIZED_LENGTH]).is_err());
        assert!(Bytes::from_bytes(&data_bytes[..U32_SERIALIZED_LENGTH + 2]).is_err());
    }

    #[test]
    fn should_serialize_deserialize_bytes() {
        let data: Bytes = vec![1, 2, 3, 4, 5].into();
        bytesrepr::test_serialization_roundtrip(&data);
    }

    #[test]
    fn should_fail_to_serialize_deserialize_malicious_bytes() {
        let data: Bytes = vec![1, 2, 3, 4, 5].into();
        let mut serialized = data.to_bytes().expect("should serialize data");
        serialized = serialized[..serialized.len() - 1].to_vec();
        let res: Result<(_, &[u8]), Error> = Bytes::from_bytes(&serialized);
        assert_eq!(res.unwrap_err(), Error::EarlyEndOfStream);
    }

    #[test]
    fn should_serialize_deserialize_bytes_and_keep_rem() {
        let data: Bytes = vec![1, 2, 3, 4, 5].into();
        let expected_rem: Vec<u8> = vec![6, 7, 8, 9, 10];
        let mut serialized = data.to_bytes().expect("should serialize data");
        serialized.extend(&expected_rem);
        let (deserialized, rem): (Bytes, &[u8]) =
            FromBytes::from_bytes(&serialized).expect("should deserialize data");
        assert_eq!(data, deserialized);
        assert_eq!(&rem, &expected_rem);
    }

    #[test]
    fn should_ser_de_human_readable() {
        let truth = vec![0xde, 0xad, 0xbe, 0xef];

        let bytes_ser: Bytes = truth.clone().into();

        let json_object = serde_json::to_value(bytes_ser).unwrap();
        assert_eq!(json_object, json!("DEaDBeef"));

        let bytes_de: Bytes = serde_json::from_value(json_object).unwrap();
        assert_eq!(bytes_de, Bytes::from(truth));
    }

    #[test]
    fn should_ser_de_readable() {
        let truth: Bytes = TRUTH.into();
        assert_tokens(&truth.readable(), &[Token::Str("DEaDBeef")]);
    }

    #[test]
    fn should_ser_de_compact() {
        let truth: Bytes = TRUTH.into();
        assert_tokens(&truth.compact(), &[Token::Bytes(TRUTH)]);
    }
}

#[cfg(test)]
pub mod gens {
    use super::Bytes;
    use proptest::{
        collection::{vec, SizeRange},
        prelude::*,
    };

    pub fn bytes_arb(size: impl Into<SizeRange>) -> impl Strategy<Value = Bytes> {
        vec(any::<u8>(), size).prop_map(Bytes::from)
    }
}
