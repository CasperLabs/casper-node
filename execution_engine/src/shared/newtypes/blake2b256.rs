/// The number of bytes in a Blake2b hash
use std::{array::TryFromSliceError, convert::TryFrom};

use blake2::{
    digest::{Update, VariableOutput},
    VarBlake2b,
};
use serde::{Deserialize, Serialize};

use casper_types::{
    bytesrepr::{self, FromBytes, ToBytes},
    check_summed_hex,
};

/// Represents a 32-byte BLAKE2b hash digest
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct Blake2bHash([u8; Blake2bHash::LENGTH]);

impl Blake2bHash {
    pub const LENGTH: usize = 32;

    /// Creates a 32-byte BLAKE2b hash digest from a given a piece of data
    pub fn new(data: &[u8]) -> Self {
        let mut ret = [0u8; Blake2bHash::LENGTH];
        // NOTE: Safe to unwrap here because our digest length is constant and valid
        let mut hasher = VarBlake2b::new(Blake2bHash::LENGTH).unwrap();
        hasher.update(data);
        hasher.finalize_variable(|hash| ret.clone_from_slice(hash));
        Blake2bHash(ret)
    }

    /// Returns the underlying BLAKE2b hash bytes
    pub fn value(&self) -> [u8; Blake2bHash::LENGTH] {
        self.0
    }

    /// Converts the underlying BLAKE2b hash digest array to a `Vec`
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl core::fmt::Display for Blake2bHash {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "Blake2bHash({})", check_summed_hex::encode(self))
    }
}

impl core::fmt::Debug for Blake2bHash {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}", self)
    }
}

impl From<[u8; Blake2bHash::LENGTH]> for Blake2bHash {
    fn from(arr: [u8; Blake2bHash::LENGTH]) -> Self {
        Blake2bHash(arr)
    }
}

impl<'a> TryFrom<&'a [u8]> for Blake2bHash {
    type Error = TryFromSliceError;

    fn try_from(slice: &[u8]) -> Result<Blake2bHash, Self::Error> {
        <[u8; Blake2bHash::LENGTH]>::try_from(slice).map(Blake2bHash)
    }
}

impl AsRef<[u8]> for Blake2bHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Into<[u8; Blake2bHash::LENGTH]> for Blake2bHash {
    fn into(self) -> [u8; Blake2bHash::LENGTH] {
        self.0
    }
}

impl ToBytes for Blake2bHash {
    #[inline(always)]
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        self.0.to_bytes()
    }

    #[inline(always)]
    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
    }
}

impl FromBytes for Blake2bHash {
    #[inline(always)]
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        FromBytes::from_bytes(bytes).map(|(arr, rem)| (Blake2bHash(arr), rem))
    }
}
