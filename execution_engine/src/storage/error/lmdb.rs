use std::sync;

use lmdb as lmdb_external;
use thiserror::Error;

use casper_types::bytesrepr;

use crate::storage::{error::in_memory, global_state::CommitError};

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Lmdb(#[from] lmdb_external::Error),

    #[error(transparent)]
    BytesRepr(#[from] bytesrepr::Error),

    #[error("Another thread panicked while holding a lock")]
    Poison,

    #[error(transparent)]
    CommitError(#[from] CommitError),
}

impl wasmi::HostError for Error {}

impl<T> From<sync::PoisonError<T>> for Error {
    fn from(_error: sync::PoisonError<T>) -> Self {
        Error::Poison
    }
}

impl From<in_memory::Error> for Error {
    fn from(error: in_memory::Error) -> Self {
        match error {
            in_memory::Error::BytesRepr(error) => Error::BytesRepr(error),
            in_memory::Error::Poison => Error::Poison,
        }
    }
}
