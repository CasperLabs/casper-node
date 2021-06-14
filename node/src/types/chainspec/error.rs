use std::{io, path::PathBuf};

use thiserror::Error;
use uint::FromDecStrErr;

use casper_types::account::ACCOUNT_HASH_LENGTH;

use crate::utils::ReadFileError;

/// Error returned by the ChainspecLoader.
#[derive(Debug, Error)]
pub enum Error {
    /// Error while decoding the chainspec from TOML format.
    #[error("decoding from TOML error: {0}")]
    DecodingFromToml(#[from] toml::de::Error),

    /// Error while decoding Motes from a decimal format.
    #[error("decoding motes from base-10 error: {0}")]
    DecodingMotes(#[from] FromDecStrErr),

    /// Error loading the chainspec.
    #[error("could not load chainspec: {0}")]
    LoadChainspec(ReadFileError),

    /// Error loading the upgrade point.
    #[error("could not load upgrade point: {0}")]
    LoadUpgradePoint(ReadFileError),

    /// Error loading the chainspec accounts.
    #[error("could not load chainspec accounts: {0}")]
    LoadChainspecAccounts(#[from] ChainspecAccountsLoadError),

    /// Error loading the global state update.
    #[error("could not load the global state update: {0}")]
    LoadGlobalStateUpgrade(#[from] GlobalStateUpdateLoadError),

    /// Failed to read the given directory.
    #[error("failed to read dir {}: {error}", dir.display())]
    ReadDir {
        /// The directory which could not be read.
        dir: PathBuf,
        /// The underlying error.
        error: io::Error,
    },

    /// No subdirectory representing a semver version was found in the given directory.
    #[error("failed to get a valid version from subdirs in {}", dir.display())]
    NoVersionSubdirFound {
        /// The searched directory.
        dir: PathBuf,
    },
}

/// Error loading chainspec accounts file.
#[derive(Debug, Error)]
pub enum ChainspecAccountsLoadError {
    /// Error loading the accounts file.
    #[error("could not load accounts: {0}")]
    LoadAccounts(#[from] ReadFileError),

    /// Error while decoding the chainspec accounts from TOML format.
    #[error("decoding from TOML error: {0}")]
    DecodingFromToml(#[from] toml::de::Error),

    /// Error while decoding a chainspec account's key hash from hex format.
    #[error("decoding from hex error: {0}")]
    DecodingFromHex(#[from] base16::DecodeError),

    /// Error while decoding Motes from a decimal format.
    #[error("decoding motes from base-10 error: {0}")]
    DecodingMotes(#[from] FromDecStrErr),

    /// Decoding a chainspec account's key hash yielded an invalid length byte array.
    #[error("expected hash length of {}, got {0}", ACCOUNT_HASH_LENGTH)]
    InvalidHashLength(usize),

    /// Error while decoding a chainspec account's key hash from base-64 format.
    #[error("crypto module error: {0}")]
    Crypto(#[from] crate::crypto::Error),
}

/// Error loading global state update file.
#[derive(Debug, Error)]
pub enum GlobalStateUpdateLoadError {
    /// Error loading the accounts file.
    #[error("could not load the file: {0}")]
    LoadFile(#[from] ReadFileError),

    /// Error while decoding the chainspec accounts from TOML format.
    #[error("decoding from TOML error: {0}")]
    DecodingFromToml(#[from] toml::de::Error),

    /// Error while decoding a serialized value from a base64 encoded string.
    #[error("decoding from base64 error: {0}")]
    DecodingFromBase64(#[from] base64::DecodeError),

    /// Error while decoding a key from formatted string.
    #[error("decoding from formatted string error: {0}")]
    DecodingKeyFromStr(String),
}
