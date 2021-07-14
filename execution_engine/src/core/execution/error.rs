use parity_wasm::elements;
use thiserror::Error;

use casper_types::{
    account::{AddKeyFailure, RemoveKeyFailure, SetThresholdFailure, UpdateKeyFailure},
    bytesrepr, system, AccessRights, ApiError, CLType, CLValueError, ContractHash,
    ContractPackageHash, ContractVersionKey, ContractWasmHash, Key, URef,
};

use crate::{
    core::resolvers::error::ResolverError,
    shared::{gas_counter::error::GasLimitError, wasm_prep, TypeMismatch},
    storage,
};

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("Interpreter error: {}", _0)]
    Interpreter(String),
    #[error("Storage error: {}", _0)]
    Storage(storage::error::Error),
    #[error("Serialization error: {}", _0)]
    BytesRepr(bytesrepr::Error),
    #[error("Named key {} not found", _0)]
    NamedKeyNotFound(String),
    #[error("Key {} not found", _0)]
    KeyNotFound(Key),
    #[error("Account {:?} not found", _0)]
    AccountNotFound(Key),
    #[error("{}", _0)]
    TypeMismatch(TypeMismatch),
    #[error("Invalid access rights: {}", required)]
    InvalidAccess { required: AccessRights },
    #[error("Forged reference: {}", _0)]
    ForgedReference(URef),
    #[error("URef not found: {}", _0)]
    URefNotFound(URef),
    #[error("Function not found: {}", _0)]
    FunctionNotFound(String),
    #[error("{}", _0)]
    ParityWasm(elements::Error),
    #[error("WASM optimizer error")]
    WasmOptimizer,
    #[error("Out of gas error")]
    GasLimit,
    #[error("Return")]
    Ret(Vec<URef>),
    #[error("{}", _0)]
    Rng(String),
    #[error("Resolver error: {}", _0)]
    Resolver(ResolverError),
    /// Reverts execution with a provided status
    #[error("{}", _0)]
    Revert(ApiError),
    #[error("{}", _0)]
    AddKeyFailure(AddKeyFailure),
    #[error("{}", _0)]
    RemoveKeyFailure(RemoveKeyFailure),
    #[error("{}", _0)]
    UpdateKeyFailure(UpdateKeyFailure),
    #[error("{}", _0)]
    SetThresholdFailure(SetThresholdFailure),
    #[error("{}", _0)]
    SystemContract(system::Error),
    #[error("Deployment authorization failure")]
    DeploymentAuthorizationFailure,
    #[error("Expected return value")]
    ExpectedReturnValue,
    #[error("Unexpected return value")]
    UnexpectedReturnValue,
    #[error("Invalid context")]
    InvalidContext,
    #[error("Incompatible protocol major version. Expected version {expected} but actual version is {actual}")]
    IncompatibleProtocolMajorVersion { expected: u32, actual: u32 },
    #[error("{0}")]
    CLValue(CLValueError),
    #[error("Host buffer is empty")]
    HostBufferEmpty,
    #[error("Unsupported WASM start")]
    UnsupportedWasmStart,
    #[error("No active contract versions for contract package")]
    NoActiveContractVersions(ContractPackageHash),
    #[error("Invalid contract version: {}", _0)]
    InvalidContractVersion(ContractVersionKey),
    #[error("No such method: {}", _0)]
    NoSuchMethod(String),
    #[error("Wasm preprocessing error: {}", _0)]
    WasmPreprocessing(wasm_prep::PreprocessingError),
    #[error("Unexpected Key length. Expected length {expected} but actual length is {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },
    #[error("Key is not a URef: {}", _0)]
    KeyIsNotAURef(Key),
    #[error("Unexpected variant of a stored value")]
    UnexpectedStoredValueVariant,
    #[error("A locked contract cannot be upgraded")]
    LockedContract(ContractPackageHash),
    #[error("Invalid contract package: {}", _0)]
    InvalidContractPackage(ContractPackageHash),
    #[error("Invalid contract: {}", _0)]
    InvalidContract(ContractHash),
    #[error("Invalid contract WASM: {}", _0)]
    InvalidContractWasm(ContractWasmHash),
    #[error("Missing argument: {name}")]
    MissingArgument { name: String },
}

impl From<wasm_prep::PreprocessingError> for Error {
    fn from(error: wasm_prep::PreprocessingError) -> Self {
        Error::WasmPreprocessing(error)
    }
}

impl From<pwasm_utils::OptimizerError> for Error {
    fn from(_optimizer_error: pwasm_utils::OptimizerError) -> Self {
        Error::WasmOptimizer
    }
}

impl From<GasLimitError> for Error {
    fn from(_: GasLimitError) -> Self {
        Error::GasLimit
    }
}

impl Error {
    pub fn type_mismatch(expected: CLType, found: CLType) -> Error {
        Error::TypeMismatch(TypeMismatch {
            expected: format!("{:?}", expected),
            found: format!("{:?}", found),
        })
    }
}

impl wasmi::HostError for Error {}

impl From<wasmi::Error> for Error {
    fn from(error: wasmi::Error) -> Self {
        match error
            .as_host_error()
            .and_then(|host_error| host_error.downcast_ref::<Error>())
        {
            Some(error) => error.clone(),
            None => Error::Interpreter(error.into()),
        }
    }
}

impl From<storage::error::Error> for Error {
    fn from(e: storage::error::Error) -> Self {
        Error::Storage(e)
    }
}

impl From<bytesrepr::Error> for Error {
    fn from(e: bytesrepr::Error) -> Self {
        Error::BytesRepr(e)
    }
}

impl From<elements::Error> for Error {
    fn from(e: elements::Error) -> Self {
        Error::ParityWasm(e)
    }
}

impl From<ResolverError> for Error {
    fn from(err: ResolverError) -> Self {
        Error::Resolver(err)
    }
}

impl From<AddKeyFailure> for Error {
    fn from(err: AddKeyFailure) -> Self {
        Error::AddKeyFailure(err)
    }
}

impl From<RemoveKeyFailure> for Error {
    fn from(err: RemoveKeyFailure) -> Self {
        Error::RemoveKeyFailure(err)
    }
}

impl From<UpdateKeyFailure> for Error {
    fn from(err: UpdateKeyFailure) -> Self {
        Error::UpdateKeyFailure(err)
    }
}

impl From<SetThresholdFailure> for Error {
    fn from(err: SetThresholdFailure) -> Self {
        Error::SetThresholdFailure(err)
    }
}

impl From<system::Error> for Error {
    fn from(error: system::Error) -> Self {
        Error::SystemContract(error)
    }
}

impl From<CLValueError> for Error {
    fn from(e: CLValueError) -> Self {
        Error::CLValue(e)
    }
}
