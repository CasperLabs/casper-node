use alloc::string::String;
use core::fmt::{self, Debug, Display, Formatter};

use ed25519_dalek::ed25519::Error as SignatureError;

/// Cryptographic errors.
#[derive(Debug)]
pub enum Error {
    /// Error resulting from creating or using asymmetric key types.
    AsymmetricKey(String),

    /// Error resulting when decoding a type from a hex-encoded representation.
    FromHex(base16::DecodeError),

    /// Error resulting when decoding a type from a base64 representation.
    FromBase64(base64::DecodeError),

    /// Signature error.
    SignatureError(SignatureError),
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(self, formatter)
    }
}

impl From<base16::DecodeError> for Error {
    fn from(error: base16::DecodeError) -> Self {
        Error::FromHex(error)
    }
}

impl From<SignatureError> for Error {
    fn from(error: SignatureError) -> Self {
        Error::SignatureError(error)
    }
}
