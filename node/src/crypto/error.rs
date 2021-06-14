use std::result;

use pem::PemError;
use signature::Error as SignatureError;
use thiserror::Error;

use crate::utils::{ReadFileError, WriteFileError};
use casper_types::crypto;

/// A specialized `std::result::Result` type for cryptographic errors.
pub type Result<T> = result::Result<T, Error>;

/// Cryptographic errors.
#[derive(Debug, Error)]
pub enum Error {
    /// Error resulting from creating or using asymmetric key types.
    #[error("asymmetric key error: {0}")]
    AsymmetricKey(String),

    /// Error resulting when decoding a type from a hex-encoded representation.
    #[error("parsing from hex: {0}")]
    FromHex(#[from] base16::DecodeError),

    /// Attempted to coerce an array which is not 32 bytes into a [`Digest`].
    #[error("Digest must be 32 bytes, was {actual_byte_length}")]
    DigestMustBe32Bytes {
        /// The actual number of bytes we were trying to convert into a [`Digest`].
        actual_byte_length: usize,
    },

    /// Error trying to read a secret key.
    #[error("secret key load failed: {0}")]
    SecretKeyLoad(ReadFileError),

    /// Error trying to read a public key.
    #[error("public key load failed: {0}")]
    PublicKeyLoad(ReadFileError),

    /// Error resulting when decoding a type from a base64 representation.
    #[error("decoding error: {0}")]
    FromBase64(#[from] base64::DecodeError),

    /// Pem format error.
    #[error("pem error: {0}")]
    FromPem(String),

    /// DER format error.
    #[error("der error: {0}")]
    FromDer(#[from] derp::Error),

    /// Error trying to write a secret key.
    #[error("secret key save failed: {0}")]
    SecretKeySave(WriteFileError),

    /// Error trying to write a public key.
    #[error("public key save failed: {0}")]
    PublicKeySave(WriteFileError),

    /// Error trying to manipulate the system key.
    #[error("invalid operation on system key: {0}")]
    System(String),

    /// Error related to the underlying signature crate.
    #[error("error in signature")]
    Signature(SignatureError),

    /// Error in getting random bytes from the system's preferred random number source.
    #[error("failed to get random bytes: {0}")]
    GetRandomBytes(#[from] getrandom::Error),
}

impl From<PemError> for Error {
    fn from(error: PemError) -> Self {
        Error::FromPem(error.to_string())
    }
}

impl From<crypto::Error> for Error {
    fn from(error: crypto::Error) -> Self {
        match error {
            crypto::Error::AsymmetricKey(string) => Error::AsymmetricKey(string),
            crypto::Error::FromHex(error) => Error::FromHex(error),
            crypto::Error::FromBase64(error) => Error::FromBase64(error),
            crypto::Error::SignatureError(error) => Error::Signature(error),
        }
    }
}
