//! Cryptographic hash type and function.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::{
    array::TryFromSliceError,
    convert::{TryFrom, TryInto},
    fmt::{self, Debug, Display, Formatter},
};

use blake2::{
    digest::{Update, VariableOutput},
    VarBlake2b,
};
use datasize::DataSize;
#[cfg(test)]
use rand::Rng;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use casper_execution_engine::shared::newtypes::Blake2bHash;
use casper_types::{
    bytesrepr::{self, FromBytes, ToBytes},
    check_summed_hex::{self, CheckSummedHex, CheckSummedHexForm},
};

use super::Error;
#[cfg(test)]
use crate::testing::TestRng;

/// The hash digest; a wrapped `u8` array.
#[derive(
    Copy,
    Clone,
    DataSize,
    Ord,
    PartialOrd,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    JsonSchema,
)]
#[serde(deny_unknown_fields)]
#[schemars(with = "String", description = "Check-summed hex-encoded hash digest.")]
pub struct Digest(
    #[serde(with = "CheckSummedHexForm::<[u8; Digest::LENGTH]>")]
    #[schemars(skip, with = "String")]
    [u8; Digest::LENGTH],
);

impl Digest {
    /// Length of `Digest` in bytes.
    pub const LENGTH: usize = 32;

    /// Returns a copy of the wrapped `u8` array.
    pub fn to_array(&self) -> [u8; Digest::LENGTH] {
        self.0
    }

    /// Returns a copy of the wrapped `u8` array as a `Vec`
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Returns a `Digest` parsed from a hex-encoded `Digest`.
    pub fn from_hex<T: AsRef<[u8]>>(hex_input: T) -> Result<Self, Error> {
        let bytes = check_summed_hex::decode(&hex_input)?;
        let slice: [u8; Self::LENGTH] =
            bytes.try_into().map_err(|_| Error::DigestMustBe32Bytes {
                actual_byte_length: hex_input.as_ref().len(),
            })?;
        Ok(Digest(slice))
    }

    /// Generates a random instance using a `TestRng`.
    #[cfg(test)]
    pub fn random(rng: &mut TestRng) -> Self {
        Digest(rng.gen::<[u8; Digest::LENGTH]>())
    }
}

impl AsRef<[u8]> for Digest {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<[u8; Digest::LENGTH]> for Digest {
    fn from(inner: [u8; Digest::LENGTH]) -> Self {
        Digest(inner)
    }
}

impl TryFrom<&[u8]> for Digest {
    type Error = TryFromSliceError;

    fn try_from(slice: &[u8]) -> Result<Digest, Self::Error> {
        <[u8; Digest::LENGTH]>::try_from(slice).map(Digest)
    }
}

impl Debug for Digest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", check_summed_hex::encode(&self.0))
    }
}

impl Display for Digest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:10}", check_summed_hex::encode(&self.0))
    }
}

impl ToBytes for Digest {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        self.0.to_bytes()
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
    }
}

impl FromBytes for Digest {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        FromBytes::from_bytes(bytes).map(|(inner, remainder)| (Digest(inner), remainder))
    }
}

/// Returns the hash of `data`.
pub fn hash<T: AsRef<[u8]>>(data: T) -> Digest {
    let mut result = [0; Digest::LENGTH];

    let mut hasher = VarBlake2b::new(Digest::LENGTH).expect("should create hasher");
    hasher.update(data);
    hasher.finalize_variable(|slice| {
        result.copy_from_slice(slice);
    });
    Digest(result)
}

impl From<Digest> for Blake2bHash {
    fn from(digest: Digest) -> Self {
        let digest_bytes = digest.to_array();
        Blake2bHash::from(digest_bytes)
    }
}

impl From<Blake2bHash> for Digest {
    fn from(blake2bhash: Blake2bHash) -> Self {
        let bytes = blake2bhash.value();
        Digest::from(bytes)
    }
}

#[cfg(test)]
mod test {
    use std::iter::{self, FromIterator};

    use super::*;

    #[test]
    fn blake2b_hash_known() {
        let inputs_and_digests = [
            (
                "",
                "0e5751c026e543B2e8AB2EB06099Daa1d1e5df47778F7787faAB45cdf12Fe3A8",
            ),
            (
                "abc",
                "BddD813C634239723171eF3fEE98579B94964E3bB1cB3E427262C8c068d52319",
            ),
            (
                "The quick brown fox jumps over the lazy dog",
                "01718cEc35Cd3D796dd00020e0BfEcb473AD23457d063b75EFf29c0ffA2e58a9",
            ),
        ];
        for (known_input, expected_digest) in &inputs_and_digests {
            let known_input: &[u8] = known_input.as_ref();
            assert_eq!(*expected_digest, format!("{:?}", hash(known_input)));
        }
    }

    #[test]
    fn from_valid_hex_should_succeed() {
        for char in "abcdefABCDEF0123456789".chars() {
            let input = String::from_iter(iter::repeat(char).take(64));
            assert!(Digest::from_hex(input).is_ok());
        }
    }

    #[test]
    fn from_hex_invalid_length_should_fail() {
        for len in &[2_usize, 62, 63, 65, 66] {
            let input = String::from_iter(iter::repeat('f').take(*len));
            assert!(Digest::from_hex(input).is_err());
        }
    }

    #[test]
    fn from_hex_invalid_char_should_fail() {
        for char in "g %-".chars() {
            let input = String::from_iter(iter::repeat('f').take(63).chain(iter::once(char)));
            assert!(Digest::from_hex(input).is_err());
        }
    }

    #[test]
    fn should_display_digest_in_hex() {
        let hash = Digest([0u8; 32]);
        let hash_hex = format!("{:?}", hash);
        assert_eq!(
            hash_hex,
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn should_print_digest_hex() {
        let hash = Digest([10u8; 32]);
        assert_eq!(
            check_summed_hex::encode(&hash),
            "0a0A0a0A0A0A0A0a0A0a0A0a0A0A0a0A0A0a0a0a0a0A0A0a0a0a0A0A0A0A0A0A"
        )
    }

    #[test]
    fn bytesrepr_roundtrip() {
        let mut rng = crate::new_rng();
        let hash = Digest::random(&mut rng);
        bytesrepr::test_serialization_roundtrip(&hash);
    }
}
