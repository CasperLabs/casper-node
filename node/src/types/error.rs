//! Errors that may be emitted by methods for common types.

use std::collections::BTreeMap;

use casper_types::{PublicKey, U512};
use thiserror::Error;

use crate::types::block::EraReport;

/// An error that can arise when creating a block from a finalized block and other components
#[derive(Error, Debug)]
pub enum BlockCreationError {
    /// `EraEnd`s need both an `EraReport` present and a map of the next era validator weights.
    /// If one of them is not present while trying to construct an `EraEnd` we must emit an
    /// error.
    #[error(
        "Cannot create EraEnd unless we have both an EraReport and next era validators. \
         Era report: {maybe_era_report:?}, \
         Next era validator weights: {maybe_next_era_validator_weights:?}"
    )]
    CouldNotCreateEraEnd {
        /// An optional `EraReport` we tried to use to construct an `EraEnd`
        maybe_era_report: Option<EraReport>,
        /// An optional map of the next era validator weights used to construct an `EraEnd`
        maybe_next_era_validator_weights: Option<BTreeMap<PublicKey, U512>>,
    },

    /// Wrapper of [`blake2::digest::InvalidOutputSize`]; occurs when trying to construct an
    /// `EraEnd`.
    #[error(transparent)]
    Blake2bDigestInvalidOutputSize(#[from] blake2::digest::InvalidOutputSize),
}
