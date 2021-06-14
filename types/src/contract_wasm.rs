use alloc::{format, string::String, vec::Vec};
use core::{
    array::TryFromSliceError,
    convert::TryFrom,
    fmt::{self, Debug, Display, Formatter},
};

use datasize::DataSize;
#[cfg(feature = "std")]
use schemars::{gen::SchemaGenerator, schema::Schema, JsonSchema};
use serde::{de::Error as SerdeError, Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    account,
    account::TryFromSliceForAccountHashError,
    bytesrepr::{Bytes, Error, FromBytes, ToBytes},
    check_summed_hex, uref, CLType, CLTyped, HashAddr,
};

const CONTRACT_WASM_MAX_DISPLAY_LEN: usize = 16;
const KEY_HASH_LENGTH: usize = 32;
const WASM_STRING_PREFIX: &str = "contract-wasm-";

/// Associated error type of `TryFrom<&[u8]>` for `ContractWasmHash`.
#[derive(Debug)]
pub struct TryFromSliceForContractHashError(());

#[derive(Debug)]
pub enum FromStrError {
    InvalidPrefix,
    Hex(base16::DecodeError),
    Account(TryFromSliceForAccountHashError),
    Hash(TryFromSliceError),
    AccountHash(account::FromStrError),
    URef(uref::FromStrError),
}

impl From<base16::DecodeError> for FromStrError {
    fn from(error: base16::DecodeError) -> Self {
        FromStrError::Hex(error)
    }
}

impl From<TryFromSliceForAccountHashError> for FromStrError {
    fn from(error: TryFromSliceForAccountHashError) -> Self {
        FromStrError::Account(error)
    }
}

impl From<TryFromSliceError> for FromStrError {
    fn from(error: TryFromSliceError) -> Self {
        FromStrError::Hash(error)
    }
}

impl From<account::FromStrError> for FromStrError {
    fn from(error: account::FromStrError) -> Self {
        FromStrError::AccountHash(error)
    }
}

impl From<uref::FromStrError> for FromStrError {
    fn from(error: uref::FromStrError) -> Self {
        FromStrError::URef(error)
    }
}

impl Display for FromStrError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            FromStrError::InvalidPrefix => write!(f, "invalid prefix"),
            FromStrError::Hex(error) => write!(f, "decode from hex: {}", error),
            FromStrError::Account(error) => write!(f, "account from string error: {:?}", error),
            FromStrError::Hash(error) => write!(f, "hash from string error: {}", error),
            FromStrError::AccountHash(error) => {
                write!(f, "account hash from string error: {:?}", error)
            }
            FromStrError::URef(error) => write!(f, "uref from string error: {:?}", error),
        }
    }
}

/// A newtype wrapping a `HashAddr` which is the raw bytes of
/// the ContractWasmHash
#[derive(DataSize, Default, PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ContractWasmHash(HashAddr);

impl ContractWasmHash {
    /// Constructs a new `ContractWasmHash` from the raw bytes of the contract wasm hash.
    pub const fn new(value: HashAddr) -> ContractWasmHash {
        ContractWasmHash(value)
    }

    /// Returns the raw bytes of the contract hash as an array.
    pub fn value(&self) -> HashAddr {
        self.0
    }

    /// Returns the raw bytes of the contract hash as a `slice`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Formats the `ContractWasmHash` for users getting and putting.
    pub fn to_formatted_string(&self) -> String {
        format!(
            "{}{}",
            WASM_STRING_PREFIX,
            check_summed_hex::encode(&self.0),
        )
    }

    /// Parses a string formatted as per `Self::to_formatted_string()` into a
    /// `ContractWasmHash`.
    pub fn from_formatted_str(input: &str) -> Result<Self, FromStrError> {
        let remainder = input
            .strip_prefix(WASM_STRING_PREFIX)
            .ok_or(FromStrError::InvalidPrefix)?;
        let bytes = HashAddr::try_from(check_summed_hex::decode(remainder)?.as_ref())?;
        Ok(ContractWasmHash(bytes))
    }
}

impl Display for ContractWasmHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", check_summed_hex::encode(&self.0))
    }
}

impl Debug for ContractWasmHash {
    fn fmt(&self, f: &mut Formatter) -> core::fmt::Result {
        write!(f, "ContractWasmHash({})", check_summed_hex::encode(&self.0))
    }
}
impl CLTyped for ContractWasmHash {
    fn cl_type() -> CLType {
        CLType::ByteArray(KEY_HASH_LENGTH as u32)
    }
}

impl ToBytes for ContractWasmHash {
    #[inline(always)]
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.0.to_bytes()
    }

    #[inline(always)]
    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
    }
}

impl FromBytes for ContractWasmHash {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (bytes, rem) = FromBytes::from_bytes(bytes)?;
        Ok((ContractWasmHash::new(bytes), rem))
    }
}

impl From<[u8; 32]> for ContractWasmHash {
    fn from(bytes: [u8; 32]) -> Self {
        ContractWasmHash(bytes)
    }
}

impl Serialize for ContractWasmHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            self.to_formatted_string().serialize(serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for ContractWasmHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let formatted_string = String::deserialize(deserializer)?;
            ContractWasmHash::from_formatted_str(&formatted_string).map_err(SerdeError::custom)
        } else {
            let bytes = HashAddr::deserialize(deserializer)?;
            Ok(ContractWasmHash(bytes))
        }
    }
}

impl AsRef<[u8]> for ContractWasmHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl TryFrom<&[u8]> for ContractWasmHash {
    type Error = TryFromSliceForContractHashError;

    fn try_from(bytes: &[u8]) -> Result<Self, TryFromSliceForContractHashError> {
        HashAddr::try_from(bytes)
            .map(ContractWasmHash::new)
            .map_err(|_| TryFromSliceForContractHashError(()))
    }
}

impl TryFrom<&Vec<u8>> for ContractWasmHash {
    type Error = TryFromSliceForContractHashError;

    fn try_from(bytes: &Vec<u8>) -> Result<Self, Self::Error> {
        HashAddr::try_from(bytes as &[u8])
            .map(ContractWasmHash::new)
            .map_err(|_| TryFromSliceForContractHashError(()))
    }
}

#[cfg(feature = "std")]
impl JsonSchema for ContractWasmHash {
    fn schema_name() -> String {
        String::from("ContractWasmHash")
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema_object = schema.into_object();
        schema_object.metadata().description =
            Some("The hash address of the contract wasm".to_string());
        schema_object.into()
    }
}

/// A container for contract's WASM bytes.
#[derive(PartialEq, Eq, Clone, Serialize)]
pub struct ContractWasm {
    bytes: Bytes,
}

impl Debug for ContractWasm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.bytes.len() > CONTRACT_WASM_MAX_DISPLAY_LEN {
            write!(
                f,
                "ContractWasm(0x{}...)",
                check_summed_hex::encode(&self.bytes[..CONTRACT_WASM_MAX_DISPLAY_LEN])
            )
        } else {
            write!(
                f,
                "ContractWasm(0x{})",
                check_summed_hex::encode(&self.bytes)
            )
        }
    }
}

impl ContractWasm {
    /// Creates new WASM object from bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        ContractWasm {
            bytes: bytes.into(),
        }
    }

    /// Consumes instance of [`ContractWasm`] and returns its bytes.
    pub fn take_bytes(self) -> Vec<u8> {
        self.bytes.into()
    }

    /// Returns a slice of contained WASM bytes.
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

impl ToBytes for ContractWasm {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.bytes.to_bytes()
    }

    fn serialized_length(&self) -> usize {
        self.bytes.serialized_length()
    }
}

impl FromBytes for ContractWasm {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (bytes, rem1) = FromBytes::from_bytes(bytes)?;
        Ok((ContractWasm { bytes }, rem1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_debug_repr_of_short_wasm() {
        const SIZE: usize = 8;
        let wasm_bytes = vec![0; SIZE];
        let contract_wasm = ContractWasm::new(wasm_bytes);
        // String output is less than the bytes itself
        assert_eq!(
            format!("{:?}", contract_wasm),
            "ContractWasm(0x0000000000000000)"
        );
    }

    #[test]
    fn test_debug_repr_of_long_wasm() {
        const SIZE: usize = 65;
        let wasm_bytes = vec![0; SIZE];
        let contract_wasm = ContractWasm::new(wasm_bytes);
        // String output is less than the bytes itself
        assert_eq!(
            format!("{:?}", contract_wasm),
            "ContractWasm(0x00000000000000000000000000000000...)"
        );
    }

    #[test]
    fn contract_wasm_hash_from_slice() {
        let bytes: Vec<u8> = (0..32).collect();
        let contract_hash =
            HashAddr::try_from(&bytes[..]).expect("should create contract wasm hash");
        let contract_hash = ContractWasmHash::new(contract_hash);
        assert_eq!(&bytes, &contract_hash.as_bytes());
    }

    #[test]
    fn contract_wasm_hash_from_str() {
        let contract_hash = ContractWasmHash([3; 32]);
        let encoded = contract_hash.to_formatted_string();
        let decoded = ContractWasmHash::from_formatted_str(&encoded).unwrap();
        assert_eq!(contract_hash, decoded);

        let invalid_prefix =
            "contractwasm-0000000000000000000000000000000000000000000000000000000000000000";
        assert!(ContractWasmHash::from_formatted_str(invalid_prefix).is_err());

        let short_addr =
            "contract-wasm-00000000000000000000000000000000000000000000000000000000000000";
        assert!(ContractWasmHash::from_formatted_str(short_addr).is_err());

        let long_addr =
            "contract-wasm-000000000000000000000000000000000000000000000000000000000000000000";
        assert!(ContractWasmHash::from_formatted_str(long_addr).is_err());

        let invalid_hex =
            "contract-wasm-000000000000000000000000000000000000000000000000000000000000000g";
        assert!(ContractWasmHash::from_formatted_str(invalid_hex).is_err());
    }

    #[test]
    fn contract_wasm_hash_serde_roundtrip() {
        let contract_hash = ContractWasmHash([255; 32]);
        let serialized = bincode::serialize(&contract_hash).unwrap();
        let deserialized = bincode::deserialize(&serialized).unwrap();
        assert_eq!(contract_hash, deserialized)
    }

    #[test]
    fn contract_wasm_hash_json_roundtrip() {
        let contract_hash = ContractWasmHash([255; 32]);
        let json_string = serde_json::to_string_pretty(&contract_hash).unwrap();
        let decoded = serde_json::from_str(&json_string).unwrap();
        assert_eq!(contract_hash, decoded)
    }
}
