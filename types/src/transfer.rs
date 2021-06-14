// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use alloc::{format, string::String, vec::Vec};
use core::{
    array::TryFromSliceError,
    convert::TryFrom,
    fmt::{self, Debug, Display, Formatter},
};

use datasize::DataSize;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
#[cfg(feature = "std")]
use schemars::{gen::SchemaGenerator, schema::Schema, JsonSchema};
use serde::{de::Error as SerdeError, Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    account::AccountHash,
    bytesrepr::{self, FromBytes, ToBytes},
    check_summed_hex, CLType, CLTyped, URef, U512,
};

/// The length of a deploy hash.
pub const DEPLOY_HASH_LENGTH: usize = 32;
/// The length of a transfer address.
pub const TRANSFER_ADDR_LENGTH: usize = 32;
pub(super) const TRANSFER_ADDR_FORMATTED_STRING_PREFIX: &str = "transfer-";

/// A newtype wrapping a [`[u8; DEPLOY_HASH_LENGTH]`] which is the raw bytes of the deploy hash.
#[derive(DataSize, Default, PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct DeployHash([u8; DEPLOY_HASH_LENGTH]);

impl DeployHash {
    /// Constructs a new `DeployHash` instance from the raw bytes of a deploy hash.
    pub const fn new(value: [u8; DEPLOY_HASH_LENGTH]) -> DeployHash {
        DeployHash(value)
    }

    /// Returns the raw bytes of the deploy hash as an array.
    pub fn value(&self) -> [u8; DEPLOY_HASH_LENGTH] {
        self.0
    }

    /// Returns the raw bytes of the deploy hash as a `slice`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(feature = "std")]
impl JsonSchema for DeployHash {
    fn schema_name() -> String {
        String::from("DeployHash")
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema_object = schema.into_object();
        schema_object.metadata().description = Some("Hex-encoded deploy hash.".to_string());
        schema_object.into()
    }
}

impl ToBytes for DeployHash {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        self.0.to_bytes()
    }

    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
    }
}

impl FromBytes for DeployHash {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        <[u8; DEPLOY_HASH_LENGTH]>::from_bytes(bytes)
            .map(|(inner, remainder)| (DeployHash(inner), remainder))
    }
}

impl Serialize for DeployHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            check_summed_hex::encode(&self.0).serialize(serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for DeployHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = if deserializer.is_human_readable() {
            let hex_string = String::deserialize(deserializer)?;
            let vec_bytes = base16::decode(hex_string.as_bytes()).map_err(SerdeError::custom)?;
            <[u8; DEPLOY_HASH_LENGTH]>::try_from(vec_bytes.as_ref()).map_err(SerdeError::custom)?
        } else {
            <[u8; DEPLOY_HASH_LENGTH]>::deserialize(deserializer)?
        };
        Ok(DeployHash(bytes))
    }
}

impl Distribution<DeployHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> DeployHash {
        DeployHash::new(rng.gen())
    }
}

/// Represents a transfer from one purse to another
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    /// Deploy that created the transfer
    pub deploy_hash: DeployHash,
    /// Account from which transfer was executed
    pub from: AccountHash,
    /// Account to which funds are transferred
    pub to: Option<AccountHash>,
    /// Source purse
    pub source: URef,
    /// Target purse
    pub target: URef,
    /// Transfer amount
    pub amount: U512,
    /// Gas
    pub gas: U512,
    /// User-defined id
    pub id: Option<u64>,
}

impl Transfer {
    /// Creates a [`Transfer`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        deploy_hash: DeployHash,
        from: AccountHash,
        to: Option<AccountHash>,
        source: URef,
        target: URef,
        amount: U512,
        gas: U512,
        id: Option<u64>,
    ) -> Self {
        Transfer {
            deploy_hash,
            from,
            to,
            source,
            target,
            amount,
            gas,
            id,
        }
    }
}

impl FromBytes for Transfer {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (deploy_hash, rem) = FromBytes::from_bytes(bytes)?;
        let (from, rem) = AccountHash::from_bytes(rem)?;
        let (to, rem) = <Option<AccountHash>>::from_bytes(rem)?;
        let (source, rem) = URef::from_bytes(rem)?;
        let (target, rem) = URef::from_bytes(rem)?;
        let (amount, rem) = U512::from_bytes(rem)?;
        let (gas, rem) = U512::from_bytes(rem)?;
        let (id, rem) = <Option<u64>>::from_bytes(rem)?;
        Ok((
            Transfer {
                deploy_hash,
                from,
                to,
                source,
                target,
                amount,
                gas,
                id,
            },
            rem,
        ))
    }
}

impl ToBytes for Transfer {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        result.append(&mut self.deploy_hash.to_bytes()?);
        result.append(&mut self.from.to_bytes()?);
        result.append(&mut self.to.to_bytes()?);
        result.append(&mut self.source.to_bytes()?);
        result.append(&mut self.target.to_bytes()?);
        result.append(&mut self.amount.to_bytes()?);
        result.append(&mut self.gas.to_bytes()?);
        result.append(&mut self.id.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.deploy_hash.serialized_length()
            + self.from.serialized_length()
            + self.to.serialized_length()
            + self.source.serialized_length()
            + self.target.serialized_length()
            + self.amount.serialized_length()
            + self.gas.serialized_length()
            + self.id.serialized_length()
    }
}

/// Error returned when decoding a `TransferAddr` from a formatted string.
#[derive(Debug)]
pub enum FromStrError {
    /// The prefix is invalid.
    InvalidPrefix,
    /// The address is not valid hex.
    Hex(base16::DecodeError),
    /// The slice is the wrong length.
    Length(TryFromSliceError),
}

impl From<base16::DecodeError> for FromStrError {
    fn from(error: base16::DecodeError) -> Self {
        FromStrError::Hex(error)
    }
}

impl From<TryFromSliceError> for FromStrError {
    fn from(error: TryFromSliceError) -> Self {
        FromStrError::Length(error)
    }
}

impl Display for FromStrError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            FromStrError::InvalidPrefix => write!(f, "prefix is not 'transfer-'"),
            FromStrError::Hex(error) => {
                write!(f, "failed to decode address portion from hex: {}", error)
            }
            FromStrError::Length(error) => write!(f, "address portion is wrong length: {}", error),
        }
    }
}

/// A newtype wrapping a [`[u8; TRANSFER_ADDR_LENGTH]`] which is the raw bytes of the transfer
/// address.
#[derive(DataSize, Default, PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy)]
pub struct TransferAddr([u8; TRANSFER_ADDR_LENGTH]);

impl TransferAddr {
    /// Constructs a new `TransferAddr` instance from the raw bytes.
    pub const fn new(value: [u8; TRANSFER_ADDR_LENGTH]) -> TransferAddr {
        TransferAddr(value)
    }

    /// Returns the raw bytes of the transfer address as an array.
    pub fn value(&self) -> [u8; TRANSFER_ADDR_LENGTH] {
        self.0
    }

    /// Returns the raw bytes of the transfer address as a `slice`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Formats the `TransferAddr` as a prefixed, hex-encoded string.
    pub fn to_formatted_string(&self) -> String {
        format!(
            "{}{}",
            TRANSFER_ADDR_FORMATTED_STRING_PREFIX,
            check_summed_hex::encode(&self.0),
        )
    }

    /// Parses a string formatted as per `Self::to_formatted_string()` into a `TransferAddr`.
    pub fn from_formatted_str(input: &str) -> Result<Self, FromStrError> {
        let remainder = input
            .strip_prefix(TRANSFER_ADDR_FORMATTED_STRING_PREFIX)
            .ok_or(FromStrError::InvalidPrefix)?;
        let bytes = <[u8; TRANSFER_ADDR_LENGTH]>::try_from(base16::decode(remainder)?.as_ref())?;
        Ok(TransferAddr(bytes))
    }
}

#[cfg(feature = "std")]
impl JsonSchema for TransferAddr {
    fn schema_name() -> String {
        String::from("TransferAddr")
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema_object = schema.into_object();
        schema_object.metadata().description = Some("Hex-encoded transfer address.".to_string());
        schema_object.into()
    }
}

impl Serialize for TransferAddr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            self.to_formatted_string().serialize(serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for TransferAddr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let formatted_string = String::deserialize(deserializer)?;
            TransferAddr::from_formatted_str(&formatted_string).map_err(SerdeError::custom)
        } else {
            let bytes = <[u8; TRANSFER_ADDR_LENGTH]>::deserialize(deserializer)?;
            Ok(TransferAddr(bytes))
        }
    }
}

// impl TryFrom<&[u8]> for AccountHash {
//     type Error = TryFromSliceForAccountHashError;
//
//     fn try_from(bytes: &[u8]) -> Result<Self, TryFromSliceForAccountHashError> {
//         [u8; TRANSFER_ADDR_LENGTH]::try_from(bytes)
//             .map(AccountHash::new)
//             .map_err(|_| TryFromSliceForAccountHashError(()))
//     }
// }
//
// impl TryFrom<&alloc::vec::Vec<u8>> for AccountHash {
//     type Error = TryFromSliceForAccountHashError;
//
//     fn try_from(bytes: &Vec<u8>) -> Result<Self, Self::Error> {
//         [u8; TRANSFER_ADDR_LENGTH]::try_from(bytes as &[u8])
//             .map(AccountHash::new)
//             .map_err(|_| TryFromSliceForAccountHashError(()))
//     }
// }

impl Display for TransferAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", check_summed_hex::encode(&self.0))
    }
}

impl Debug for TransferAddr {
    fn fmt(&self, f: &mut Formatter) -> core::fmt::Result {
        write!(f, "TransferAddr({})", check_summed_hex::encode(&self.0))
    }
}

impl CLTyped for TransferAddr {
    fn cl_type() -> CLType {
        CLType::ByteArray(TRANSFER_ADDR_LENGTH as u32)
    }
}

impl ToBytes for TransferAddr {
    #[inline(always)]
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        self.0.to_bytes()
    }

    #[inline(always)]
    fn serialized_length(&self) -> usize {
        self.0.serialized_length()
    }
}

impl FromBytes for TransferAddr {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (bytes, remainder) = FromBytes::from_bytes(bytes)?;
        Ok((TransferAddr::new(bytes), remainder))
    }
}

impl AsRef<[u8]> for TransferAddr {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Distribution<TransferAddr> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> TransferAddr {
        TransferAddr::new(rng.gen())
    }
}

/// Generators for [`Transfer`]
#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::prelude::{prop::option, Arbitrary, Strategy};

    use crate::{
        deploy_info::gens::{account_hash_arb, deploy_hash_arb},
        gens::{u512_arb, uref_arb},
        Transfer,
    };

    /// Creates an arbitrary [`Transfer`]
    pub fn transfer_arb() -> impl Strategy<Value = Transfer> {
        (
            deploy_hash_arb(),
            account_hash_arb(),
            option::of(account_hash_arb()),
            uref_arb(),
            uref_arb(),
            u512_arb(),
            u512_arb(),
            option::of(<u64>::arbitrary()),
        )
            .prop_map(|(deploy_hash, from, to, source, target, amount, gas, id)| {
                Transfer {
                    deploy_hash,
                    from,
                    to,
                    source,
                    target,
                    amount,
                    gas,
                    id,
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::bytesrepr;

    use super::*;

    proptest! {
        #[test]
        fn test_serialization_roundtrip(transfer in gens::transfer_arb()) {
            bytesrepr::test_serialization_roundtrip(&transfer)
        }
    }

    #[test]
    fn transfer_addr_from_str() {
        let transfer_address = TransferAddr([4; 32]);
        let encoded = transfer_address.to_formatted_string();
        let decoded = TransferAddr::from_formatted_str(&encoded).unwrap();
        assert_eq!(transfer_address, decoded);

        let invalid_prefix =
            "transfe-0000000000000000000000000000000000000000000000000000000000000000";
        assert!(TransferAddr::from_formatted_str(invalid_prefix).is_err());

        let invalid_prefix =
            "transfer0000000000000000000000000000000000000000000000000000000000000000";
        assert!(TransferAddr::from_formatted_str(invalid_prefix).is_err());

        let short_addr = "transfer-00000000000000000000000000000000000000000000000000000000000000";
        assert!(TransferAddr::from_formatted_str(short_addr).is_err());

        let long_addr =
            "transfer-000000000000000000000000000000000000000000000000000000000000000000";
        assert!(TransferAddr::from_formatted_str(long_addr).is_err());

        let invalid_hex =
            "transfer-000000000000000000000000000000000000000000000000000000000000000g";
        assert!(TransferAddr::from_formatted_str(invalid_hex).is_err());
    }

    #[test]
    fn transfer_addr_serde_roundtrip() {
        let transfer_address = TransferAddr([255; 32]);
        let serialized = bincode::serialize(&transfer_address).unwrap();
        let decoded = bincode::deserialize(&serialized).unwrap();
        assert_eq!(transfer_address, decoded);
    }

    #[test]
    fn transfer_addr_json_roundtrip() {
        let transfer_address = TransferAddr([255; 32]);
        let json_string = serde_json::to_string_pretty(&transfer_address).unwrap();
        let decoded = serde_json::from_str(&json_string).unwrap();
        assert_eq!(transfer_address, decoded);
    }
}
