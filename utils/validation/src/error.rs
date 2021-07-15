use casper_types::bytesrepr;
use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Deserialize(#[from] serde_json::Error),
    #[error("missing file stem in: {0}")]
    NoStem(PathBuf),
    #[error("unsupported file format at {0}")]
    UnsupportedFormat(PathBuf),
    #[error("file {0} lacks extension")]
    NoExtension(PathBuf),
    #[error(transparent)]
    Bytesrepr(#[from] bytesrepr::Error),
}
