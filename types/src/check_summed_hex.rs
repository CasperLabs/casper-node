//! Check-summed hex encoding following an [EIP-55][1]-like scheme.
//!
//! Contains a [serde] helper trait [CheckSummedHex] which is based on [`hex_buffer_serde::Hex`][2].
//!
//! [1]: https://eips.ethereum.org/EIPS/eip-55
//! [2]: https://docs.rs/hex-buffer-serde/0.3.0/hex_buffer_serde/trait.Hex.html
use alloc::{borrow::Cow, string::String, vec::Vec};
use core::{convert::TryFrom, fmt, marker::PhantomData};

use blake2::{Blake2b, Digest};
use serde::{
    de::{Error as DeError, Unexpected, Visitor},
    Deserializer, Serializer,
};

/// Takes a slice of bytes and breaks it up into a vector of *nibbles* (ie, 4-bit values)
/// represented as `u8`s.
fn bytes_to_nibbles(input: &(impl AsRef<[u8]> + ?Sized)) -> Vec<u8> {
    let bytes: &[u8] = input.as_ref();
    let mut output_nibbles = Vec::with_capacity(bytes.len() * 2);
    for b in bytes {
        output_nibbles.push(b >> 4);
        output_nibbles.push(b & 0x0f);
    }
    output_nibbles
}

/// Computes a Blake2b hash outputs it as a vector of *nibbles* (ie, 4-bit values) represented as
/// `usize`s.
fn blake2b_hash(data: impl AsRef<[u8]>) -> Vec<u8> {
    let mut hasher = Blake2b::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

const HEX_CHARS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

/// Encodes bytes as hexadecimal with mixed-case based checksums following a scheme similar to
/// [EIP-55][1].
///
/// Key differences:
///   - Works on any length of data, not just 20-byte addresses
///   - Uses Blake2b hashes rather than Keccak
///   - Hash nibbles based on input rather than hex encoding of input
///
/// [1]: https://eips.ethereum.org/EIPS/eip-55
pub fn encode(input: &(impl AsRef<[u8]> + ?Sized)) -> String {
    let input_nibbles = bytes_to_nibbles(input);
    let hash_nibbles = bytes_to_nibbles(&blake2b_hash(input));
    let hash_nibble_count = hash_nibbles.len();
    let mut hex_output_string = String::with_capacity(input_nibbles.len());
    for idx in 0..input_nibbles.len() {
        let c = HEX_CHARS[input_nibbles[idx] as usize];
        if hash_nibbles[idx % hash_nibble_count] <= 7 {
            hex_output_string.extend(c.to_uppercase())
        } else {
            hex_output_string.extend(c.to_lowercase())
        }
    }
    hex_output_string
}

fn string_is_uppercase(input: &(impl AsRef<[u8]> + ?Sized)) -> bool {
    let bytes: &[u8] = input.as_ref();
    bytes
        .iter()
        .map(|byte| *byte as char)
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_uppercase())
}

fn string_is_lowercase(input: &(impl AsRef<[u8]> + ?Sized)) -> bool {
    let bytes: &[u8] = input.as_ref();
    bytes
        .iter()
        .map(|byte| *byte as char)
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_lowercase())
}

/// Decodes a mixed-case hexadecimal string, verifying that it conforms to the checksum scheme
/// similar to scheme in [EIP-55][1].
///
/// Key differences:
///   - Works on any length of data, not just 20-byte addresses
///   - Uses Blake2b hashes rather than Keccak
///   - Hash nibbles based on input rather than hex encoding of input
///   - Hash nibbles are XORed with input nibbles
///
/// For backward compatibility: if the hex string is all uppercase or all lowercase, the check is
/// skipped.
///
/// [1]: https://eips.ethereum.org/EIPS/eip-55
pub fn decode(input: &(impl AsRef<[u8]> + ?Sized)) -> Result<Vec<u8>, base16::DecodeError> {
    let bytes = base16::decode(input)?;
    // If the string was all uppercase or all lower case, don't verify the checksum.
    // This is to support legacy clients.
    // Otherwise perform the check as below.
    if !(string_is_uppercase(input) || string_is_lowercase(input)) {
        let checksum_hex_bytes = encode(&bytes).into_bytes();
        let input_string_bytes = input.as_ref();
        for idx in 0..input_string_bytes.len() {
            if checksum_hex_bytes[idx] != input_string_bytes[idx] {
                return Err(base16::DecodeError::InvalidByte {
                    index: idx,
                    byte: input_string_bytes[idx],
                });
            }
        }
    }
    Ok(bytes)
}

/// Provides check-summed hex-encoded (de)serialization for `serde`, an following an
/// [EIP-55][1]-like scheme.
///
/// Note that the trait is automatically implemented for types that implement [`AsRef`]`<[u8]>` and
/// [`TryFrom`]`<&[u8]>`.
///
/// [1]: https://eips.ethereum.org/EIPS/eip-55
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub trait CheckSummedHex<T> {
    /// Error returned on unsuccessful deserialization.
    type Error: fmt::Display;

    /// Converts the value into bytes. This is used for serialization.
    ///
    /// The returned buffer can be either borrowed from the type, or created by the method.
    fn create_bytes(value: &T) -> Cow<'_, [u8]>;

    /// Creates a value from the byte slice.
    fn from_bytes(bytes: &[u8]) -> Result<T, Self::Error>;

    /// Serializes the value for `serde`. This method is not meant to be overridden.
    fn serialize<S: Serializer>(value: &T, serializer: S) -> Result<S::Ok, S::Error> {
        let value = Self::create_bytes(value);
        if serializer.is_human_readable() {
            serializer.serialize_str(&encode(&value))
        } else {
            serializer.serialize_bytes(value.as_ref())
        }
    }

    /// Deserializes a value using `serde`. This method is not meant to be overridden.
    ///
    /// If the deserializer is [human-readable][hr] (e.g., JSON or TOML), this method
    /// expects a hex-encoded string. Otherwise, the method expects a byte array.
    ///
    /// [hr]: serde::Serializer::is_human_readable()
    fn deserialize<'de, D>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CheckSummedHexVisitor;

        impl<'de> Visitor<'de> for CheckSummedHexVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("check-summed hex-encoded byte array")
            }

            fn visit_str<E: DeError>(self, value: &str) -> Result<Self::Value, E> {
                decode(value).map_err(|_| E::invalid_type(Unexpected::Str(value), &self))
            }

            // See the `deserializing_flattened_field` test for an example why this is needed.
            fn visit_bytes<E: DeError>(self, value: &[u8]) -> Result<Self::Value, E> {
                Ok(value.to_vec())
            }
        }

        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("byte array")
            }

            fn visit_bytes<E: DeError>(self, value: &[u8]) -> Result<Self::Value, E> {
                Ok(value.to_vec())
            }
        }

        let maybe_bytes = if deserializer.is_human_readable() {
            deserializer.deserialize_str(CheckSummedHexVisitor)
        } else {
            deserializer.deserialize_bytes(BytesVisitor)
        };
        maybe_bytes.and_then(|bytes| Self::from_bytes(&bytes).map_err(D::Error::custom))
    }
}

/// A dummy container for use inside `#[serde(with)]` attribute if the underlying type
/// implements [`Hex`].
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
#[derive(Debug)]
pub struct CheckSummedHexForm<T>(PhantomData<T>);

impl<T, E> CheckSummedHex<T> for CheckSummedHexForm<T>
where
    T: AsRef<[u8]> + for<'a> TryFrom<&'a [u8], Error = E>,
    E: fmt::Display,
{
    type Error = E;

    fn create_bytes(buffer: &T) -> Cow<'_, [u8]> {
        Cow::Borrowed(buffer.as_ref())
    }

    fn from_bytes(bytes: &[u8]) -> Result<T, Self::Error> {
        T::try_from(bytes)
    }
}

#[cfg(test)]
#[allow(renamed_and_removed_lints, clippy::unknown_clippy_lints)]
// ^ `map_err_ignore` is newer than MSRV, and `clippy::unknown_clippy_lints` is removed
// since Rust 1.51.
//
// Tests taken from https://github.com/slowli/hex-buffer-serde/blob/8ff1523898497d1e4f65781bcb076070109c9df3/src/var_len.rs#L138
mod tests {
    use super::*;

    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use alloc::{
        borrow::ToOwned,
        string::{String, ToString},
        vec,
    };
    use core::array::TryFromSliceError;

    #[derive(Debug, Serialize, Deserialize)]
    struct Buffer([u8; 8]);

    impl AsRef<[u8]> for Buffer {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }

    impl TryFrom<&[u8]> for Buffer {
        type Error = TryFromSliceError;

        fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
            <[u8; 8]>::try_from(slice).map(Buffer)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct Test {
        #[serde(with = "CheckSummedHexForm::<Buffer>")]
        buffer: Buffer,
        other_field: String,
    }

    #[test]
    fn internal_type() {
        let json = json!({ "buffer": "0001020304050607", "other_field": "abc" });
        let value: Test = serde_json::from_value(json.clone()).unwrap();
        assert!(value
            .buffer
            .0
            .iter()
            .enumerate()
            .all(|(i, &byte)| i == usize::from(byte)));

        let json_copy = serde_json::to_value(&value).unwrap();
        assert_eq!(json, json_copy);
    }

    #[test]
    fn error_reporting() {
        let bogus_jsons = vec![
            serde_json::json!({
                "buffer": "bogus",
                "other_field": "test",
            }),
            serde_json::json!({
                "buffer": "c0ffe",
                "other_field": "test",
            }),
        ];

        for bogus_json in bogus_jsons {
            let err = serde_json::from_value::<Test>(bogus_json)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("expected check-summed hex-encoded byte array"),
                "{}",
                err
            );
        }
    }

    #[test]
    fn internal_type_with_derived_serde_code() {
        // ...and here, we may use original `serde` code.
        #[derive(Serialize, Deserialize)]
        struct OriginalTest {
            buffer: Buffer,
            other_field: String,
        }

        let test = Test {
            buffer: Buffer([1; 8]),
            other_field: "a".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(test).unwrap(),
            json!({
                "buffer": "0101010101010101",
                "other_field": "a",
            })
        );

        let test = OriginalTest {
            buffer: Buffer([1; 8]),
            other_field: "a".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(test).unwrap(),
            json!({
                "buffer": [1, 1, 1, 1, 1, 1, 1, 1],
                "other_field": "a",
            })
        );
    }

    #[test]
    fn external_type() {
        #[derive(Debug, PartialEq, Eq)]
        pub struct Buffer([u8; 8]);

        struct BufferHex(());

        impl CheckSummedHex<Buffer> for BufferHex {
            type Error = &'static str;

            fn create_bytes(buffer: &Buffer) -> Cow<'_, [u8]> {
                Cow::Borrowed(&buffer.0)
            }

            fn from_bytes(bytes: &[u8]) -> Result<Buffer, Self::Error> {
                if bytes.len() == 8 {
                    let mut inner = [0; 8];
                    inner.copy_from_slice(bytes);
                    Ok(Buffer(inner))
                } else {
                    Err("invalid buffer length")
                }
            }
        }

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Test {
            #[serde(with = "BufferHex")]
            buffer: Buffer,
            other_field: String,
        }

        let json = json!({ "buffer": "0001020304050607", "other_field": "abc" });
        let value: Test = serde_json::from_value(json.clone()).unwrap();
        assert!(value
            .buffer
            .0
            .iter()
            .enumerate()
            .all(|(i, &byte)| i == usize::from(byte)));

        let json_copy = serde_json::to_value(&value).unwrap();
        assert_eq!(json, json_copy);

        // Test binary / non-human readable format.
        let buffer = bincode::serialize(&value).unwrap();
        // Conversion to hex is needed to be able to search for a pattern.
        let buffer_hex = encode(&buffer);
        // Check that the buffer is stored in the serialization compactly,
        // as original bytes.
        let needle = "0001020304050607";
        assert!(buffer_hex.contains(needle));

        let value_copy: Test = bincode::deserialize(&buffer).unwrap();
        assert_eq!(value_copy, value);
    }

    #[test]
    fn deserializing_flattened_field() {
        // The fields in the flattened structure are somehow read with
        // a human-readable `Deserializer`, even if the original `Deserializer`
        // is not human-readable.
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Inner {
            #[serde(with = "CheckSummedHexForm")]
            x: Vec<u8>,
            #[serde(with = "CheckSummedHexForm")]
            y: [u8; 16],
        }

        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Outer {
            #[serde(flatten)]
            inner: Inner,
            z: String,
        }

        let value = Outer {
            inner: Inner {
                x: vec![1; 8],
                y: [0; 16],
            },
            z: "test".to_owned(),
        };

        let bytes = serde_cbor::to_vec(&value).unwrap();
        let bytes_hex = encode(&bytes);
        // Check that byte buffers are stored in the binary form.
        assert!(bytes_hex.contains(&"01".repeat(8)));
        assert!(bytes_hex.contains(&"00".repeat(16)));
        let value_copy = serde_cbor::from_slice(&bytes).unwrap();
        assert_eq!(value, value_copy);
    }
}
