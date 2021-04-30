//! Asymmetric key types and methods on them

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    cmp::Ordering,
    convert::TryFrom,
    fmt::{self, Debug, Display, Formatter},
    hash::{Hash, Hasher},
    iter,
    marker::Copy,
};

use datasize::DataSize;
use ed25519_dalek::{
    ed25519::signature::Signature as _Signature, PUBLIC_KEY_LENGTH as ED25519_PUBLIC_KEY_LENGTH,
    SECRET_KEY_LENGTH as ED25519_SECRET_KEY_LENGTH, SIGNATURE_LENGTH as ED25519_SIGNATURE_LENGTH,
};
use hex_fmt::HexFmt;
use k256::ecdsa::{
    Signature as Secp256k1Signature, SigningKey as Secp256k1SecretKey,
    VerifyingKey as Secp256k1PublicKey,
};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
#[cfg(feature = "std")]
use schemars::{gen::SchemaGenerator, schema::Schema, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    account::AccountHash,
    bytesrepr,
    bytesrepr::{FromBytes, ToBytes, U8_SERIALIZED_LENGTH},
    crypto::Error,
    CLType, CLTyped, Tagged,
};

#[cfg(any(feature = "gens", test))]
pub mod gens;
#[cfg(test)]
mod tests;

const TAG_LENGTH: usize = U8_SERIALIZED_LENGTH;
const SYSTEM: &str = "System";
const ED25519: &str = "Ed25519";
const SECP256K1: &str = "Secp256k1";

/// Representation of a asymmetric key tag
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, FromPrimitive, ToPrimitive, Debug)]
#[repr(u8)]
pub enum AsymmetricKeyTag {
    /// Tag for system variant.
    System = 0,
    /// Tag for ed25519 variant.
    Ed25519 = 1,
    /// Tag for secp256k1 variant.
    Secp256k1 = 2,
}

impl AsymmetricKeyTag {
    fn variant_name(self) -> &'static str {
        match self {
            AsymmetricKeyTag::System => SYSTEM,
            AsymmetricKeyTag::Ed25519 => ED25519,
            AsymmetricKeyTag::Secp256k1 => SECP256K1,
        }
    }
}

impl From<AsymmetricKeyTag> for u8 {
    fn from(tag: AsymmetricKeyTag) -> Self {
        tag.to_u8().unwrap()
    }
}

impl FromBytes for AsymmetricKeyTag {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag_value, rem) = FromBytes::from_bytes(bytes)?;
        let tag = AsymmetricKeyTag::from_u8(tag_value).ok_or(bytesrepr::Error::Formatting)?;
        Ok((tag, rem))
    }
}

const SECP256K1_SECRET_KEY_LENGTH: usize = 32;
const SECP256K1_COMPRESSED_PUBLIC_KEY_LENGTH: usize = 33;
const SECP256K1_SIGNATURE_LENGTH: usize = 64;

/// Public key for system account.
pub const SYSTEM_ACCOUNT: PublicKey = PublicKey::System;

/// Operations on asymmetric cryptographic type.
pub trait AsymmetricType<'a>
where
    Self: 'a + Sized + Tagged,
    Self::Tag: Into<u8>,
    &'a Self: Into<Vec<u8>>,
{
    /// Converts `self` to hex, where the first byte represents the algorithm tag.
    fn to_hex(&'a self) -> String {
        let bytes = iter::once(self.tag().into())
            .chain(self.into())
            .collect::<Vec<u8>>();
        hex::encode(bytes)
    }

    /// Tries to decode `Self` from its hex-representation.  The hex format should be as produced
    /// by `AsymmetricType::to_hex()`.
    fn from_hex<A: AsRef<[u8]>>(input: A) -> Result<Self, Error> {
        if input.as_ref().len() < 2 {
            return Err(Error::AsymmetricKey("too short".to_string()));
        }

        let (tag_bytes, key_bytes) = input.as_ref().split_at(2);
        let mut tag_value = [0u8; 1];
        hex::decode_to_slice(tag_bytes, tag_value.as_mut())?;
        let tag = AsymmetricKeyTag::from_u8(tag_value[0]);

        match tag {
            Some(AsymmetricKeyTag::Ed25519) => {
                let bytes = hex::decode(key_bytes)?;
                Self::ed25519_from_bytes(&bytes)
            }
            Some(AsymmetricKeyTag::Secp256k1) => {
                let bytes = hex::decode(key_bytes)?;
                Self::secp256k1_from_bytes(&bytes)
            }
            Some(AsymmetricKeyTag::System) | None => Err(Error::AsymmetricKey(format!(
                "invalid tag.  Expected {} or {}, got {}",
                AsymmetricKeyTag::Ed25519.to_u64().unwrap(),
                AsymmetricKeyTag::Secp256k1.to_u64().unwrap(),
                tag_value[0]
            ))),
        }
    }

    /// Constructs a new system variant.
    fn system() -> Self;

    /// Constructs a new ed25519 variant from a byte slice.
    fn ed25519_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error>;

    /// Constructs a new secp256k1 variant from a byte slice.
    fn secp256k1_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error>;
}

/// A secret or private asymmetric key.
#[derive(DataSize)]
pub enum SecretKey {
    /// System secret key.
    System,
    /// Ed25519 secret key.
    #[data_size(skip)] // Manually verified to have no data on the heap.
    Ed25519(ed25519_dalek::SecretKey),
    /// secp256k1 secret key.
    #[data_size(skip)]
    Secp256k1(Secp256k1SecretKey),
}

impl SecretKey {
    /// The length in bytes of a system secret key.
    pub const SYSTEM_LENGTH: usize = 0;

    /// The length in bytes of an Ed25519 secret key.
    pub const ED25519_LENGTH: usize = ED25519_SECRET_KEY_LENGTH;

    /// The length in bytes of a secp256k1 secret key.
    pub const SECP256K1_LENGTH: usize = SECP256K1_SECRET_KEY_LENGTH;

    /// Constructs a new system variant.
    pub fn system() -> Self {
        SecretKey::System
    }

    /// Constructs a new ed25519 variant from a byte slice.
    pub fn ed25519_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(SecretKey::Ed25519(ed25519_dalek::SecretKey::from_bytes(
            bytes.as_ref(),
        )?))
    }

    /// Constructs a new secp256k1 variant from a byte slice.
    pub fn secp256k1_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(SecretKey::Secp256k1(Secp256k1SecretKey::from_bytes(
            bytes.as_ref(),
        )?))
    }
}

impl Debug for SecretKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "SecretKey::{}", self.tag().variant_name())
    }
}

impl Display for SecretKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        <Self as Debug>::fmt(self, formatter)
    }
}

impl Tagged for SecretKey {
    type Tag = AsymmetricKeyTag;

    fn tag(&self) -> Self::Tag {
        match self {
            SecretKey::System => AsymmetricKeyTag::System,
            SecretKey::Ed25519(_) => AsymmetricKeyTag::Ed25519,
            SecretKey::Secp256k1(_) => AsymmetricKeyTag::Secp256k1,
        }
    }
}

/// A public asymmetric key.
#[derive(Clone, DataSize, Eq, PartialEq)]
pub enum PublicKey {
    /// System public key.
    System,
    /// Ed25519 public key.
    #[data_size(skip)] // Manually verified to have no data on the heap.
    Ed25519(ed25519_dalek::PublicKey),
    /// secp256k1 public key.
    #[data_size(skip)]
    Secp256k1(Secp256k1PublicKey),
}

impl PublicKey {
    /// The length in bytes of a system public key.
    pub const SYSTEM_LENGTH: usize = 0;

    /// The length in bytes of an Ed25519 public key.
    pub const ED25519_LENGTH: usize = ED25519_PUBLIC_KEY_LENGTH;

    /// The length in bytes of a secp256k1 public key.
    pub const SECP256K1_LENGTH: usize = SECP256K1_COMPRESSED_PUBLIC_KEY_LENGTH;

    /// Creates an `AccountHash` from a given `PublicKey` instance.
    pub fn to_account_hash(&self) -> AccountHash {
        AccountHash::from(self)
    }
}

impl AsymmetricType<'_> for PublicKey {
    fn system() -> Self {
        PublicKey::System
    }

    fn ed25519_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(PublicKey::Ed25519(ed25519_dalek::PublicKey::from_bytes(
            bytes.as_ref(),
        )?))
    }

    fn secp256k1_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(PublicKey::Secp256k1(Secp256k1PublicKey::from_sec1_bytes(
            bytes.as_ref(),
        )?))
    }
}

impl From<&SecretKey> for PublicKey {
    fn from(secret_key: &SecretKey) -> PublicKey {
        match secret_key {
            SecretKey::System => PublicKey::System,
            SecretKey::Ed25519(secret_key) => PublicKey::Ed25519(secret_key.into()),
            SecretKey::Secp256k1(secret_key) => PublicKey::Secp256k1(secret_key.into()),
        }
    }
}

impl From<&PublicKey> for Vec<u8> {
    fn from(public_key: &PublicKey) -> Self {
        match public_key {
            PublicKey::System => Vec::new(),
            PublicKey::Ed25519(key) => key.to_bytes().into(),
            PublicKey::Secp256k1(key) => key.to_bytes().into(),
        }
    }
}

impl From<PublicKey> for Vec<u8> {
    fn from(public_key: PublicKey) -> Self {
        Vec::<u8>::from(&public_key)
    }
}

impl Debug for PublicKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PublicKey::{}({})",
            self.tag().variant_name(),
            HexFmt(Into::<Vec<u8>>::into(self))
        )
    }
}

impl Display for PublicKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PubKey::{}({:10})",
            self.tag().variant_name(),
            HexFmt(Into::<Vec<u8>>::into(self))
        )
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_tag = self.tag();
        let other_tag = other.tag();
        if self_tag == other_tag {
            Into::<Vec<u8>>::into(self).cmp(&Into::<Vec<u8>>::into(other))
        } else {
            self_tag.cmp(&other_tag)
        }
    }
}

// This implementation of `Hash` agrees with the derived `PartialEq`.  It's required since
// `ed25519_dalek::PublicKey` doesn't implement `Hash`.
#[allow(clippy::derive_hash_xor_eq)]
impl Hash for PublicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tag().hash(state);
        Into::<Vec<u8>>::into(self).hash(state);
    }
}

impl Tagged for PublicKey {
    type Tag = AsymmetricKeyTag;

    fn tag(&self) -> Self::Tag {
        match self {
            PublicKey::System => AsymmetricKeyTag::System,
            PublicKey::Ed25519(_) => AsymmetricKeyTag::Ed25519,
            PublicKey::Secp256k1(_) => AsymmetricKeyTag::Secp256k1,
        }
    }
}

impl ToBytes for PublicKey {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        match self {
            PublicKey::System => {
                buffer.insert(0, AsymmetricKeyTag::System.into());
            }
            PublicKey::Ed25519(public_key) => {
                buffer.insert(0, AsymmetricKeyTag::Ed25519.into());
                let ed25519_bytes = public_key.as_bytes();
                buffer.extend_from_slice(ed25519_bytes);
            }
            PublicKey::Secp256k1(public_key) => {
                buffer.insert(0, AsymmetricKeyTag::Secp256k1.into());
                let secp256k1_bytes = public_key.to_bytes();
                buffer.extend_from_slice(&secp256k1_bytes);
            }
        }
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        TAG_LENGTH
            + match self {
                PublicKey::System => Self::SYSTEM_LENGTH,
                PublicKey::Ed25519(_) => Self::ED25519_LENGTH,
                PublicKey::Secp256k1(_) => Self::SECP256K1_LENGTH,
            }
    }
}

impl FromBytes for PublicKey {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = AsymmetricKeyTag::from_bytes(bytes)?;
        match tag {
            AsymmetricKeyTag::System => Ok((PublicKey::System, remainder)),
            AsymmetricKeyTag::Ed25519 => {
                let (raw_bytes, remainder): ([u8; Self::ED25519_LENGTH], _) =
                    FromBytes::from_bytes(remainder)?;
                let public_key = Self::ed25519_from_bytes(raw_bytes)
                    .map_err(|_error| bytesrepr::Error::Formatting)?;
                Ok((public_key, remainder))
            }
            AsymmetricKeyTag::Secp256k1 => {
                let (raw_bytes, remainder): ([u8; Self::SECP256K1_LENGTH], _) =
                    FromBytes::from_bytes(remainder)?;
                let public_key = Self::secp256k1_from_bytes(raw_bytes)
                    .map_err(|_error| bytesrepr::Error::Formatting)?;
                Ok((public_key, remainder))
            }
        }
    }
}

impl Serialize for PublicKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        detail::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        detail::deserialize(deserializer)
    }
}

#[cfg(feature = "std")]
impl JsonSchema for PublicKey {
    fn schema_name() -> String {
        String::from("PublicKey")
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema_object = schema.into_object();
        schema_object.metadata().description = Some(
            "Hex-encoded cryptographic public key, including the algorithm tag prefix.".to_string(),
        );
        schema_object.into()
    }
}

impl CLTyped for PublicKey {
    fn cl_type() -> CLType {
        CLType::PublicKey
    }
}

/// A signature of given data.
#[derive(Clone, Copy, DataSize)]
pub enum Signature {
    /// System signature.  Cannot be verified.
    System,
    /// Ed25519 signature.
    #[data_size(skip)]
    Ed25519(ed25519_dalek::Signature),
    /// Secp256k1 signature.
    #[data_size(skip)]
    Secp256k1(Secp256k1Signature),
}

impl Signature {
    /// The length in bytes of a system signature,
    pub const SYSTEM_LENGTH: usize = 0;

    /// The length in bytes of an Ed25519 signature,
    pub const ED25519_LENGTH: usize = ED25519_SIGNATURE_LENGTH;

    /// The length in bytes of a secp256k1 signature
    pub const SECP256K1_LENGTH: usize = SECP256K1_SIGNATURE_LENGTH;

    /// Constructs a new Ed25519 variant from a byte array.
    pub fn ed25519(bytes: [u8; Self::ED25519_LENGTH]) -> Result<Self, Error> {
        let signature = ed25519_dalek::Signature::from_bytes(&bytes).map_err(|_| {
            Error::AsymmetricKey(format!(
                "failed to construct Ed25519 signature from {:?}",
                &bytes[..]
            ))
        })?;

        Ok(Signature::Ed25519(signature))
    }

    /// Constructs a new secp256k1 variant from a byte array.
    pub fn secp256k1(bytes: [u8; Self::SECP256K1_LENGTH]) -> Result<Self, Error> {
        let signature = Secp256k1Signature::try_from(&bytes[..]).map_err(|_| {
            Error::AsymmetricKey(format!(
                "failed to construct secp256k1 signature from {:?}",
                &bytes[..]
            ))
        })?;

        Ok(Signature::Secp256k1(signature))
    }
}

impl AsymmetricType<'_> for Signature {
    fn system() -> Self {
        Signature::System
    }

    fn ed25519_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        let signature = ed25519_dalek::Signature::from_bytes(bytes.as_ref()).map_err(|_| {
            Error::AsymmetricKey(format!(
                "failed to construct Ed25519 signature from {:?}",
                bytes.as_ref()
            ))
        })?;
        Ok(Signature::Ed25519(signature))
    }

    fn secp256k1_from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        let signature = k256::ecdsa::Signature::try_from(bytes.as_ref()).map_err(|_| {
            Error::AsymmetricKey(format!(
                "failed to construct secp256k1 signature from {:?}",
                bytes.as_ref()
            ))
        })?;
        Ok(Signature::Secp256k1(signature))
    }
}

impl Debug for Signature {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Signature::{}({})",
            self.tag().variant_name(),
            HexFmt(Into::<Vec<u8>>::into(*self))
        )
    }
}

impl Display for Signature {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Sig::{}({:10})",
            self.tag().variant_name(),
            HexFmt(Into::<Vec<u8>>::into(*self))
        )
    }
}

impl PartialOrd for Signature {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Signature {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_tag = self.tag();
        let other_tag = other.tag();
        if self_tag == other_tag {
            Into::<Vec<u8>>::into(*self).cmp(&Into::<Vec<u8>>::into(*other))
        } else {
            self_tag.cmp(&other_tag)
        }
    }
}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.tag() == other.tag() && Into::<Vec<u8>>::into(*self) == Into::<Vec<u8>>::into(*other)
    }
}

impl Eq for Signature {}

impl Hash for Signature {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tag().hash(state);
        Into::<Vec<u8>>::into(*self).hash(state);
    }
}

impl Tagged for Signature {
    type Tag = AsymmetricKeyTag;

    fn tag(&self) -> Self::Tag {
        match self {
            Signature::System => AsymmetricKeyTag::System,
            Signature::Ed25519(_) => AsymmetricKeyTag::Ed25519,
            Signature::Secp256k1(_) => AsymmetricKeyTag::Secp256k1,
        }
    }
}

impl ToBytes for Signature {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        match self {
            Signature::System => {
                buffer.insert(0, AsymmetricKeyTag::System.into());
            }
            Signature::Ed25519(signature) => {
                buffer.insert(0, AsymmetricKeyTag::Ed25519.into());
                let ed5519_bytes = signature.to_bytes();
                buffer.extend(&ed5519_bytes);
            }
            Signature::Secp256k1(signature) => {
                buffer.insert(0, AsymmetricKeyTag::Secp256k1.into());
                let secp256k1_bytes = signature.as_ref();
                buffer.extend_from_slice(secp256k1_bytes);
            }
        }
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        TAG_LENGTH
            + match self {
                Signature::System => Self::SYSTEM_LENGTH,
                Signature::Ed25519(_) => Self::ED25519_LENGTH,
                Signature::Secp256k1(_) => Self::SECP256K1_LENGTH,
            }
    }
}

impl FromBytes for Signature {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, remainder) = AsymmetricKeyTag::from_bytes(bytes)?;
        match tag {
            AsymmetricKeyTag::System => Ok((Signature::System, remainder)),
            AsymmetricKeyTag::Ed25519 => {
                let (raw_bytes, remainder): ([u8; Self::ED25519_LENGTH], _) =
                    FromBytes::from_bytes(remainder)?;
                let public_key =
                    Self::ed25519(raw_bytes).map_err(|_error| bytesrepr::Error::Formatting)?;
                Ok((public_key, remainder))
            }
            AsymmetricKeyTag::Secp256k1 => {
                let (raw_bytes, remainder): ([u8; Self::SECP256K1_LENGTH], _) =
                    FromBytes::from_bytes(remainder)?;
                let public_key =
                    Self::secp256k1(raw_bytes).map_err(|_error| bytesrepr::Error::Formatting)?;
                Ok((public_key, remainder))
            }
        }
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        detail::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        detail::deserialize(deserializer)
    }
}

impl From<&Signature> for Vec<u8> {
    fn from(signature: &Signature) -> Self {
        match signature {
            Signature::System => Vec::new(),
            Signature::Ed25519(signature) => signature.to_bytes().into(),
            Signature::Secp256k1(signature) => signature.as_ref().into(),
        }
    }
}

impl From<Signature> for Vec<u8> {
    fn from(signature: Signature) -> Self {
        Vec::<u8>::from(&signature)
    }
}

#[cfg(feature = "std")]
impl JsonSchema for Signature {
    fn schema_name() -> String {
        String::from("Signature")
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema_object = schema.into_object();
        schema_object.metadata().description = Some(
            "Hex-encoded cryptographic signature, including the algorithm tag prefix.".to_string(),
        );
        schema_object.into()
    }
}

mod detail {
    use alloc::{string::String, vec::Vec};

    use serde::{de::Error as _deError, Deserialize, Deserializer, Serialize, Serializer};

    use super::{PublicKey, Signature};
    use crate::AsymmetricType;

    /// Used to serialize and deserialize asymmetric key types where the (de)serializer is not a
    /// human-readable type.
    ///
    /// The wrapped contents are the result of calling `t_as_ref()` on the type.
    #[derive(Serialize, Deserialize)]
    pub enum AsymmetricTypeAsBytes {
        System,
        Ed25519(Vec<u8>),
        Secp256k1(Vec<u8>),
    }

    impl From<&PublicKey> for AsymmetricTypeAsBytes {
        fn from(public_key: &PublicKey) -> Self {
            match public_key {
                PublicKey::System => AsymmetricTypeAsBytes::System,
                key @ PublicKey::Ed25519(_) => AsymmetricTypeAsBytes::Ed25519(key.into()),
                key @ PublicKey::Secp256k1(_) => AsymmetricTypeAsBytes::Secp256k1(key.into()),
            }
        }
    }

    impl From<&Signature> for AsymmetricTypeAsBytes {
        fn from(signature: &Signature) -> Self {
            match signature {
                Signature::System => AsymmetricTypeAsBytes::System,
                key @ Signature::Ed25519(_) => AsymmetricTypeAsBytes::Ed25519(key.into()),
                key @ Signature::Secp256k1(_) => AsymmetricTypeAsBytes::Secp256k1(key.into()),
            }
        }
    }

    pub fn serialize<'a, T, S>(value: &'a T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: AsymmetricType<'a>,
        T::Tag: Into<u8>,
        Vec<u8>: From<&'a T>,
        S: Serializer,
        AsymmetricTypeAsBytes: From<&'a T>,
    {
        if serializer.is_human_readable() {
            return value.to_hex().serialize(serializer);
        }

        AsymmetricTypeAsBytes::from(value).serialize(serializer)
    }

    pub fn deserialize<'a, 'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: AsymmetricType<'a>,
        T::Tag: Into<u8>,
        Vec<u8>: From<&'a T>,
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let hex_string = String::deserialize(deserializer)?;
            let value = T::from_hex(hex_string.as_bytes()).map_err(D::Error::custom)?;
            return Ok(value);
        }

        let as_bytes = AsymmetricTypeAsBytes::deserialize(deserializer)?;
        match as_bytes {
            AsymmetricTypeAsBytes::System => Ok(T::system()),
            AsymmetricTypeAsBytes::Ed25519(raw_bytes) => {
                T::ed25519_from_bytes(raw_bytes).map_err(D::Error::custom)
            }
            AsymmetricTypeAsBytes::Secp256k1(raw_bytes) => {
                T::secp256k1_from_bytes(raw_bytes).map_err(D::Error::custom)
            }
        }
    }
}
