// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use std::{
    array::TryFromSliceError,
    collections::HashMap,
    error::Error as StdError,
    fmt::{self, Debug, Display, Formatter},
};

use datasize::DataSize;
use derive_more::Display;
use hex::FromHexError;
use itertools::Itertools;
use num_traits::Zero;
use once_cell::sync::Lazy;
#[cfg(test)]
use rand::{Rng, RngCore};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

use casper_execution_engine::{
    core::engine_state::{executable_deploy_item::ExecutableDeployItem, DeployItem},
    shared::motes::Motes,
};
use casper_types::{
    bytesrepr::{self, FromBytes, ToBytes},
    runtime_args,
    system::standard_payment::ARG_AMOUNT,
    AsymmetricType, ExecutionResult, PublicKey, RuntimeArgs, SecretKey, Signature, U512,
};

use super::{BlockHash, Item, Tag, TimeDiff, Timestamp};
#[cfg(test)]
use crate::testing::TestRng;
use crate::{
    components::block_proposer::DeployInfo,
    crypto,
    crypto::{
        hash::{self, Digest},
        AsymmetricKeyExt,
    },
    rpcs::docs::DocExample,
    types::chainspec::DeployConfig,
    utils::DisplayIter,
};

static DEPLOY: Lazy<Deploy> = Lazy::new(|| {
    let payment_args = runtime_args! {
        "quantity" => 1000
    };
    let payment = ExecutableDeployItem::StoredContractByName {
        name: String::from("casper-example"),
        entry_point: String::from("example-entry-point"),
        args: payment_args,
    };
    let session_args = runtime_args! {
        "amount" => 1000
    };
    let session = ExecutableDeployItem::Transfer { args: session_args };
    let serialized_body = serialize_body(&payment, &session);
    let body_hash = hash::hash(&serialized_body);

    let secret_key = SecretKey::doc_example();
    let header = DeployHeader {
        account: PublicKey::from(secret_key),
        timestamp: *Timestamp::doc_example(),
        ttl: TimeDiff::from(3_600_000),
        gas_price: 1,
        body_hash,
        dependencies: vec![DeployHash::new(Digest::from([1u8; Digest::LENGTH]))],
        chain_name: String::from("casper-example"),
    };
    let serialized_header = serialize_header(&header);
    let hash = DeployHash::new(hash::hash(&serialized_header));

    let signature = Signature::from_hex(
        "012dbf03817a51794a8e19e0724884075e6d1fbec326b766ecfa6658b41f81290da85e23b24e88b1c8d976\
            1185c961daee1adab0649912a6477bcd2e69bd91bd08"
            .as_bytes(),
    )
    .unwrap();
    let approval = Approval {
        signer: PublicKey::from(secret_key),
        signature,
    };

    Deploy {
        hash,
        header,
        payment,
        session,
        approvals: vec![approval],
        is_valid: None,
    }
});

/// A representation of the way in which a deploy failed validation checks.
#[derive(Clone, DataSize, Ord, PartialOrd, Eq, PartialEq, Hash, Debug, Error)]
pub enum DeployValidationFailure {
    /// Invalid chain name.
    #[error("invalid chain name: expected {expected}, got {got}")]
    InvalidChainName {
        /// The expected chain name.
        expected: String,
        /// The received chain name.
        got: String,
    },

    /// Too many dependencies.
    #[error("{got} dependencies exceeds limit of {max_dependencies}")]
    ExcessiveDependencies {
        /// The dependencies limit.
        max_dependencies: u8,
        /// The actual number of dependencies provided.
        got: usize,
    },

    /// Deploy is too large.
    #[error("deploy size too large: {0}")]
    ExcessiveSize(#[from] ExcessiveSizeError),

    /// Excessive time-to-live.
    #[error("time-to-live of {got} exceeds limit of {max_ttl}")]
    ExcessiveTimeToLive {
        /// The time-to-live limit.
        max_ttl: TimeDiff,
        /// The received time-to-live.
        got: TimeDiff,
    },

    /// The provided body hash does not match the actual hash of the body.
    #[error("the provided body hash does not match the actual hash of the body")]
    InvalidBodyHash,

    /// The provided deploy hash does not match the actual hash of the deploy.
    #[error("the provided hash does not match the actual hash of the deploy")]
    InvalidDeployHash,

    /// Invalid approval.
    #[error("the approval at index {index} is invalid: {error_msg}")]
    InvalidApproval {
        /// The index of the approval at fault.
        index: usize,
        /// The approval validation error.
        error_msg: String,
    },

    /// Excessive length of deploy's session args.
    #[error("serialized session code runtime args of {got} exceeds limit of {max_length}")]
    ExcessiveSessionArgsLength {
        /// The byte size limit of session arguments.
        max_length: usize,
        /// The received length of session arguments.
        got: usize,
    },

    /// Excessive length of deploy's payment args.
    #[error("serialized payment code runtime args of {got} exceeds limit of {max_length}")]
    ExcessivePaymentArgsLength {
        /// The byte size limit of payment arguments.
        max_length: usize,
        /// The received length of payment arguments.
        got: usize,
    },

    /// Missing transfer amount.
    #[error("missing transfer amount")]
    MissingTransferAmount,

    /// Invalid transfer amount.
    #[error("invalid transfer amount")]
    InvalidTransferAmount,

    /// Insufficient transfer amount.
    #[error("insufficient transfer amount; minimum: {minimum} attempted: {attempted}")]
    InsufficientTransferAmount {
        /// The minimum transfer amount.
        minimum: U512,
        /// The attempted transfer amount.
        attempted: U512,
    },
}

/// Error returned when a Deploy is too large.
#[derive(Clone, DataSize, Ord, PartialOrd, Eq, PartialEq, Hash, Debug, Error)]
#[error("deploy size of {actual_deploy_size} bytes exceeds limit of {max_deploy_size}")]
pub struct ExcessiveSizeError {
    /// The maximum permitted serialized deploy size, in bytes.
    pub max_deploy_size: u32,
    /// The serialized size of the deploy provided, in bytes.
    pub actual_deploy_size: usize,
}

/// Errors other than validation failures relating to `Deploy`s.
#[derive(Debug, Error)]
pub enum Error {
    /// Error while encoding to JSON.
    #[error("encoding to JSON: {0}")]
    EncodeToJson(#[from] serde_json::Error),

    /// Error while decoding from JSON.
    #[error("decoding from JSON: {0}")]
    DecodeFromJson(Box<dyn StdError>),

    /// Failed to get "amount" from `payment()`'s runtime args.
    #[error("invalid payment: missing \"amount\" arg")]
    InvalidPayment,
}

impl From<FromHexError> for Error {
    fn from(error: FromHexError) -> Self {
        Error::DecodeFromJson(Box::new(error))
    }
}

impl From<TryFromSliceError> for Error {
    fn from(error: TryFromSliceError) -> Self {
        Error::DecodeFromJson(Box::new(error))
    }
}

/// The cryptographic hash of a [`Deploy`](struct.Deploy.html).
#[derive(
    Copy,
    Clone,
    DataSize,
    Ord,
    PartialOrd,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    Debug,
    Default,
    JsonSchema,
)]
#[serde(deny_unknown_fields)]
#[schemars(with = "String", description = "Hex-encoded deploy hash.")]
pub struct DeployHash(#[schemars(skip)] Digest);

impl DeployHash {
    /// Constructs a new `DeployHash`.
    pub fn new(hash: Digest) -> Self {
        DeployHash(hash)
    }

    /// Returns the wrapped inner hash.
    pub fn inner(&self) -> &Digest {
        &self.0
    }

    /// Creates a random deploy hash.
    #[cfg(test)]
    pub fn random(rng: &mut TestRng) -> Self {
        let hash = Digest::random(rng);
        DeployHash(hash)
    }
}

impl Display for DeployHash {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "deploy-hash({})", self.0,)
    }
}

impl From<Digest> for DeployHash {
    fn from(digest: Digest) -> Self {
        Self(digest)
    }
}

impl AsRef<[u8]> for DeployHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
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
        Digest::from_bytes(bytes).map(|(inner, remainder)| (DeployHash(inner), remainder))
    }
}

/// The [`DeployHash`](struct.DeployHash.html) stored in a way distinguishing between WASM deploys
/// and transfers.
#[derive(
    Copy,
    Clone,
    DataSize,
    Ord,
    PartialOrd,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    Debug,
    Display,
    JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub enum DeployOrTransferHash {
    /// Hash of a deploy.
    #[display(fmt = "deploy {}", _0)]
    Deploy(DeployHash),
    /// Hash of a transfer.
    #[display(fmt = "transfer {}", _0)]
    Transfer(DeployHash),
}

impl DeployOrTransferHash {
    /// Gets the inner `DeployHash`.
    pub fn deploy_hash(&self) -> &DeployHash {
        match self {
            DeployOrTransferHash::Deploy(hash) | DeployOrTransferHash::Transfer(hash) => hash,
        }
    }

    /// Returns `true` if this is a transfer hash.
    pub fn is_transfer(&self) -> bool {
        matches!(self, DeployOrTransferHash::Transfer(_))
    }
}

impl From<DeployOrTransferHash> for DeployHash {
    fn from(dt_hash: DeployOrTransferHash) -> DeployHash {
        match dt_hash {
            DeployOrTransferHash::Deploy(hash) => hash,
            DeployOrTransferHash::Transfer(hash) => hash,
        }
    }
}

/// The header portion of a [`Deploy`](struct.Deploy.html).
#[derive(
    Clone, DataSize, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, Debug, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct DeployHeader {
    account: PublicKey,
    timestamp: Timestamp,
    ttl: TimeDiff,
    gas_price: u64,
    body_hash: Digest,
    dependencies: Vec<DeployHash>,
    chain_name: String,
}

impl DeployHeader {
    /// The account within which the deploy will be run.
    pub fn account(&self) -> &PublicKey {
        &self.account
    }

    /// When the deploy was created.
    pub fn timestamp(&self) -> Timestamp {
        self.timestamp
    }

    /// How long the deploy will stay valid.
    pub fn ttl(&self) -> TimeDiff {
        self.ttl
    }

    /// Has this deploy expired?
    pub fn expired(&self, current_instant: Timestamp) -> bool {
        let lifespan = self.timestamp + self.ttl;
        lifespan < current_instant
    }

    /// Price per gas unit for this deploy.
    pub fn gas_price(&self) -> u64 {
        self.gas_price
    }

    /// Hash of the Wasm code.
    pub fn body_hash(&self) -> &Digest {
        &self.body_hash
    }

    /// Other deploys that have to be run before this one.
    pub fn dependencies(&self) -> &Vec<DeployHash> {
        &self.dependencies
    }

    /// Which chain the deploy is supposed to be run on.
    pub fn chain_name(&self) -> &str {
        &self.chain_name
    }

    /// Determine if this deploy header has valid values based on a `DeployConfig` and timestamp.
    pub fn is_valid(&self, deploy_config: &DeployConfig, current_timestamp: Timestamp) -> bool {
        let ttl_valid = self.ttl() <= deploy_config.max_ttl;
        let timestamp_valid = self.timestamp() <= current_timestamp;
        let not_expired = !self.expired(current_timestamp);
        let num_deps_valid = self.dependencies().len() <= deploy_config.max_dependencies as usize;
        ttl_valid && timestamp_valid && not_expired && num_deps_valid
    }
}

impl DeployHeader {
    /// Returns the timestamp of when the deploy expires, i.e. `self.timestamp + self.ttl`.
    pub fn expires(&self) -> Timestamp {
        self.timestamp + self.ttl
    }
}

impl ToBytes for DeployHeader {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.account.to_bytes()?);
        buffer.extend(self.timestamp.to_bytes()?);
        buffer.extend(self.ttl.to_bytes()?);
        buffer.extend(self.gas_price.to_bytes()?);
        buffer.extend(self.body_hash.to_bytes()?);
        buffer.extend(self.dependencies.to_bytes()?);
        buffer.extend(self.chain_name.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.account.serialized_length()
            + self.timestamp.serialized_length()
            + self.ttl.serialized_length()
            + self.gas_price.serialized_length()
            + self.body_hash.serialized_length()
            + self.dependencies.serialized_length()
            + self.chain_name.serialized_length()
    }
}

impl FromBytes for DeployHeader {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (account, remainder) = PublicKey::from_bytes(bytes)?;
        let (timestamp, remainder) = Timestamp::from_bytes(remainder)?;
        let (ttl, remainder) = TimeDiff::from_bytes(remainder)?;
        let (gas_price, remainder) = u64::from_bytes(remainder)?;
        let (body_hash, remainder) = Digest::from_bytes(remainder)?;
        let (dependencies, remainder) = Vec::<DeployHash>::from_bytes(remainder)?;
        let (chain_name, remainder) = String::from_bytes(remainder)?;
        let deploy_header = DeployHeader {
            account,
            timestamp,
            ttl,
            gas_price,
            body_hash,
            dependencies,
            chain_name,
        };
        Ok((deploy_header, remainder))
    }
}

impl Display for DeployHeader {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(
            formatter,
            "deploy-header[account: {}, timestamp: {}, ttl: {}, gas_price: {}, body_hash: {}, dependencies: [{}], chain_name: {}]",
            self.account,
            self.timestamp,
            self.ttl,
            self.gas_price,
            self.body_hash,
            DisplayIter::new(self.dependencies.iter()),
            self.chain_name,
        )
    }
}

/// A struct containing a signature and the public key of the signer.
#[derive(
    Clone, DataSize, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, Debug, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    signer: PublicKey,
    signature: Signature,
}

impl Approval {
    /// Returns the public key of the approval's signer.
    pub fn signer(&self) -> &PublicKey {
        &self.signer
    }

    /// Returns the approval signature.
    pub fn signature(&self) -> &Signature {
        &self.signature
    }
}

impl Display for Approval {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "approval({})", self.signer)
    }
}

impl ToBytes for Approval {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.signer.to_bytes()?);
        buffer.extend(self.signature.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.signer.serialized_length() + self.signature.serialized_length()
    }
}

impl FromBytes for Approval {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (signer, remainder) = PublicKey::from_bytes(bytes)?;
        let (signature, remainder) = Signature::from_bytes(remainder)?;
        let approval = Approval { signer, signature };
        Ok((approval, remainder))
    }
}

/// A deploy; an item containing a smart contract along with the requester's signature(s).
#[derive(
    Clone, DataSize, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, Debug, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
    hash: DeployHash,
    header: DeployHeader,
    payment: ExecutableDeployItem,
    session: ExecutableDeployItem,
    approvals: Vec<Approval>,
    #[serde(skip)]
    is_valid: Option<Result<(), DeployValidationFailure>>,
}

impl Deploy {
    /// Constructs a new signed `Deploy`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timestamp: Timestamp,
        ttl: TimeDiff,
        gas_price: u64,
        dependencies: Vec<DeployHash>,
        chain_name: String,
        payment: ExecutableDeployItem,
        session: ExecutableDeployItem,
        secret_key: &SecretKey,
        account: Option<PublicKey>,
    ) -> Deploy {
        let serialized_body = serialize_body(&payment, &session);
        let body_hash = hash::hash(&serialized_body);

        let account = account.unwrap_or_else(|| PublicKey::from(secret_key));

        // Remove duplicates.
        let dependencies = dependencies.into_iter().unique().collect();
        let header = DeployHeader {
            account,
            timestamp,
            ttl,
            gas_price,
            body_hash,
            dependencies,
            chain_name,
        };
        let serialized_header = serialize_header(&header);
        let hash = DeployHash::new(hash::hash(&serialized_header));

        let mut deploy = Deploy {
            hash,
            header,
            payment,
            session,
            approvals: vec![],
            is_valid: None,
        };

        deploy.sign(secret_key);
        deploy
    }

    /// Adds a signature of this deploy's hash to its approvals.
    pub fn sign(&mut self, secret_key: &SecretKey) {
        let signer = PublicKey::from(secret_key);
        let signature = crypto::sign(&self.hash, secret_key, &signer);
        let approval = Approval { signer, signature };
        self.approvals.push(approval);
    }

    /// Returns the `DeployHash` identifying this `Deploy`.
    pub fn id(&self) -> &DeployHash {
        &self.hash
    }

    /// Returns a reference to the `DeployHeader` of this `Deploy`.
    pub fn header(&self) -> &DeployHeader {
        &self.header
    }

    /// Returns the `DeployHeader` of this `Deploy`.
    pub fn take_header(self) -> DeployHeader {
        self.header
    }

    /// Returns the `ExecutableDeployItem` for payment code.
    pub fn payment(&self) -> &ExecutableDeployItem {
        &self.payment
    }

    /// Returns the `ExecutableDeployItem` for session code.
    pub fn session(&self) -> &ExecutableDeployItem {
        &self.session
    }

    /// Returns the `Approval`s for this deploy.
    pub fn approvals(&self) -> &[Approval] {
        &self.approvals
    }

    /// Returns the hash of this deploy wrapped in `DeployOrTransferHash`.
    pub fn deploy_or_transfer_hash(&self) -> DeployOrTransferHash {
        if self.session.is_transfer() {
            DeployOrTransferHash::Transfer(self.hash)
        } else {
            DeployOrTransferHash::Deploy(self.hash)
        }
    }

    /// Returns the `DeployInfo`.
    pub fn deploy_info(&self) -> Result<DeployInfo, Error> {
        let header = self.header().clone();
        let size = self.serialized_length();
        let payment_amount = if self.session().is_transfer() {
            // TODO: we need a non-zero value constant for wasm-less transfer cost.
            Motes::zero()
        } else {
            let payment_item = self.payment().clone();

            // In the happy path for a payment we expect:
            // - args to exist
            // - contain "amount"
            // - be a valid U512 value.
            let value = payment_item
                .args()
                .get(ARG_AMOUNT)
                .ok_or(Error::InvalidPayment)?;
            let value = value
                .clone()
                .into_t::<U512>()
                .map_err(|_| Error::InvalidPayment)?;
            Motes::new(value)
        };
        Ok(DeployInfo {
            header,
            size,
            payment_amount,
        })
    }

    /// Returns true if the serialized size of the deploy is not greater than `max_deploy_size`.
    pub fn is_valid_size(&self, max_deploy_size: u32) -> Result<(), ExcessiveSizeError> {
        let deploy_size = self.serialized_length();
        if deploy_size > max_deploy_size as usize {
            return Err(ExcessiveSizeError {
                max_deploy_size,
                actual_deploy_size: deploy_size,
            });
        }
        Ok(())
    }

    /// Returns true if and only if:
    ///   * the deploy hash is correct (should be the hash of the header), and
    ///   * the body hash is correct (should be the hash of the body), and
    ///   * all approvals are valid signatures of the deploy hash
    pub fn is_valid(&mut self) -> Result<(), DeployValidationFailure> {
        match self.is_valid.as_ref() {
            None => {
                let validity = validate_deploy(self);
                self.is_valid = Some(validity.clone());
                validity
            }
            Some(validity) => validity.clone(),
        }
    }

    /// Returns true if and only if:
    ///   * the chain_name is correct,
    ///   * the configured parameters are complied with,
    ///   * the deploy is valid
    ///
    /// Note: if everything else checks out, calls the computationally expensive `is_valid` method.
    pub fn is_acceptable(
        &mut self,
        chain_name: &str,
        config: &DeployConfig,
    ) -> Result<(), DeployValidationFailure> {
        self.is_valid_size(config.max_deploy_size)?;

        let header = self.header();
        if header.chain_name() != chain_name {
            info!(
                deploy_hash = %self.id(),
                deploy_header = %header,
                chain_name = %header.chain_name(),
                "invalid chain identifier"
            );
            return Err(DeployValidationFailure::InvalidChainName {
                expected: chain_name.to_string(),
                got: header.chain_name().to_string(),
            });
        }

        if header.dependencies().len() > config.max_dependencies as usize {
            info!(
                deploy_hash = %self.id(),
                deploy_header = %header,
                max_dependencies = %config.max_dependencies,
                "deploy dependency ceiling exceeded"
            );
            return Err(DeployValidationFailure::ExcessiveDependencies {
                max_dependencies: config.max_dependencies,
                got: header.dependencies().len(),
            });
        }

        if header.ttl() > config.max_ttl {
            info!(
                deploy_hash = %self.id(),
                deploy_header = %header,
                max_ttl = %config.max_ttl,
                "deploy ttl excessive"
            );
            return Err(DeployValidationFailure::ExcessiveTimeToLive {
                max_ttl: config.max_ttl,
                got: header.ttl(),
            });
        }

        let payment_args_length = self.payment().args().serialized_length();
        if payment_args_length > config.payment_args_max_length as usize {
            info!(
                payment_args_length,
                payment_args_max_length = config.payment_args_max_length,
                "payment args excessive"
            );
            return Err(DeployValidationFailure::ExcessivePaymentArgsLength {
                max_length: config.payment_args_max_length as usize,
                got: payment_args_length,
            });
        }

        let session_args_length = self.session().args().serialized_length();
        if session_args_length > config.session_args_max_length as usize {
            info!(
                session_args_length,
                session_args_max_length = config.session_args_max_length,
                "session args excessive"
            );
            return Err(DeployValidationFailure::ExcessiveSessionArgsLength {
                max_length: config.session_args_max_length as usize,
                got: session_args_length,
            });
        }

        if self.session().is_transfer() {
            let item = self.session().clone();
            let attempted = item
                .args()
                .get(ARG_AMOUNT)
                .ok_or(DeployValidationFailure::MissingTransferAmount)?
                .clone()
                .into_t::<U512>()
                .map_err(|_| DeployValidationFailure::InvalidTransferAmount)?;
            let minimum = U512::from(config.native_transfer_minimum_motes);
            if attempted < minimum {
                return Err(DeployValidationFailure::InsufficientTransferAmount {
                    minimum,
                    attempted,
                });
            }
        }

        self.is_valid()
    }

    /// Generates a random instance using a `TestRng`.
    #[cfg(test)]
    pub fn random(rng: &mut TestRng) -> Self {
        let timestamp = Timestamp::random(rng);
        let ttl = TimeDiff::from(rng.gen_range(60_000..3_600_000));
        let gas_price = rng.gen_range(1..100);

        let dependencies = vec![
            DeployHash::new(hash::hash(rng.next_u64().to_le_bytes())),
            DeployHash::new(hash::hash(rng.next_u64().to_le_bytes())),
            DeployHash::new(hash::hash(rng.next_u64().to_le_bytes())),
        ];
        let chain_name = String::from("casper-example");

        let payment = rng.gen();
        let session = rng.gen();

        let secret_key = SecretKey::random(rng);

        Deploy::new(
            timestamp,
            ttl,
            gas_price,
            dependencies,
            chain_name,
            payment,
            session,
            &secret_key,
            None,
        )
    }
}

impl DocExample for Deploy {
    fn doc_example() -> &'static Self {
        &*DEPLOY
    }
}

fn serialize_header(header: &DeployHeader) -> Vec<u8> {
    header
        .to_bytes()
        .unwrap_or_else(|error| panic!("should serialize deploy header: {}", error))
}

fn serialize_body(payment: &ExecutableDeployItem, session: &ExecutableDeployItem) -> Vec<u8> {
    let mut buffer = payment
        .to_bytes()
        .unwrap_or_else(|error| panic!("should serialize payment code: {}", error));
    buffer.extend(
        session
            .to_bytes()
            .unwrap_or_else(|error| panic!("should serialize session code: {}", error)),
    );
    buffer
}

// Computationally expensive validity check for a given deploy instance, including
// asymmetric_key signing verification.
fn validate_deploy(deploy: &Deploy) -> Result<(), DeployValidationFailure> {
    let serialized_body = serialize_body(&deploy.payment, &deploy.session);
    let body_hash = hash::hash(&serialized_body);
    if body_hash != deploy.header.body_hash {
        warn!(?deploy, ?body_hash, "invalid deploy body hash");
        return Err(DeployValidationFailure::InvalidBodyHash);
    }

    let serialized_header = serialize_header(&deploy.header);
    let hash = DeployHash::new(hash::hash(&serialized_header));
    if hash != deploy.hash {
        warn!(?deploy, ?hash, "invalid deploy hash");
        return Err(DeployValidationFailure::InvalidDeployHash);
    }

    // We don't need to check for an empty set here. EE checks that the correct number and weight of
    // signatures are provided when executing the deploy, so all we need to do here is check that
    // any provided signatures are valid.
    for (index, approval) in deploy.approvals.iter().enumerate() {
        if let Err(error) = crypto::verify(&deploy.hash, &approval.signature, &approval.signer) {
            warn!(?deploy, "failed to verify approval {}: {}", index, error);
            return Err(DeployValidationFailure::InvalidApproval {
                index,
                error_msg: error.to_string(),
            });
        }
    }

    Ok(())
}

impl Item for Deploy {
    type Id = DeployHash;

    const TAG: Tag = Tag::Deploy;
    const ID_IS_COMPLETE_ITEM: bool = false;

    fn id(&self) -> Self::Id {
        *self.id()
    }
}

impl Display for Deploy {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(
            formatter,
            "deploy[{}, {}, payment_code: {}, session_code: {}, approvals: {}]",
            self.hash,
            self.header,
            self.payment,
            self.session,
            DisplayIter::new(self.approvals.iter())
        )
    }
}

impl From<Deploy> for DeployItem {
    fn from(deploy: Deploy) -> Self {
        let address = deploy.header().account().to_account_hash();
        let authorization_keys = deploy
            .approvals()
            .iter()
            .map(|approval| approval.signer().to_account_hash())
            .collect();

        DeployItem::new(
            address,
            deploy.session().clone(),
            deploy.payment().clone(),
            deploy.header().gas_price(),
            authorization_keys,
            casper_types::DeployHash::new(deploy.id().inner().to_array()),
        )
    }
}

/// The deploy mutable metadata.
///
/// Currently a stop-gap measure to associate an immutable deploy with additional metadata. Holds
/// execution results.
#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct DeployMetadata {
    /// The block hashes of blocks containing the related deploy, along with the results of
    /// executing the related deploy in the context of one or more blocks.
    pub execution_results: HashMap<BlockHash, ExecutionResult>,
}

impl ToBytes for Deploy {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.header.to_bytes()?);
        buffer.extend(self.hash.to_bytes()?);
        buffer.extend(self.payment.to_bytes()?);
        buffer.extend(self.session.to_bytes()?);
        buffer.extend(self.approvals.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.header.serialized_length()
            + self.hash.serialized_length()
            + self.payment.serialized_length()
            + self.session.serialized_length()
            + self.approvals.serialized_length()
    }
}

impl FromBytes for Deploy {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (header, remainder) = DeployHeader::from_bytes(bytes)?;
        let (hash, remainder) = DeployHash::from_bytes(remainder)?;
        let (payment, remainder) = ExecutableDeployItem::from_bytes(remainder)?;
        let (session, remainder) = ExecutableDeployItem::from_bytes(remainder)?;
        let (approvals, remainder) = Vec::<Approval>::from_bytes(remainder)?;
        let maybe_valid_deploy = Deploy {
            header,
            hash,
            payment,
            session,
            approvals,
            is_valid: None,
        };
        Ok((maybe_valid_deploy, remainder))
    }
}

#[cfg(test)]
mod tests {
    use std::{iter, time::Duration};

    use casper_execution_engine::core::engine_state::MAX_PAYMENT_AMOUNT;
    use casper_types::{bytesrepr::Bytes, CLValue};

    use super::*;
    use crate::crypto::AsymmetricKeyExt;

    #[test]
    fn json_roundtrip() {
        let mut rng = crate::new_rng();
        let deploy = Deploy::random(&mut rng);
        let json_string = serde_json::to_string_pretty(&deploy).unwrap();
        let decoded = serde_json::from_str(&json_string).unwrap();
        assert_eq!(deploy, decoded);
    }

    #[test]
    fn bincode_roundtrip() {
        let mut rng = crate::new_rng();
        let deploy = Deploy::random(&mut rng);
        let serialized = bincode::serialize(&deploy).unwrap();
        let deserialized = bincode::deserialize(&serialized).unwrap();
        assert_eq!(deploy, deserialized);
    }

    #[test]
    fn bytesrepr_roundtrip() {
        let mut rng = crate::new_rng();
        let hash = DeployHash(Digest::random(&mut rng));
        bytesrepr::test_serialization_roundtrip(&hash);

        let deploy = Deploy::random(&mut rng);
        bytesrepr::test_serialization_roundtrip(deploy.header());
        bytesrepr::test_serialization_roundtrip(&deploy);
    }

    fn create_deploy(
        rng: &mut TestRng,
        ttl: TimeDiff,
        dependency_count: usize,
        chain_name: &str,
    ) -> Deploy {
        let secret_key = SecretKey::random(rng);
        let dependencies = iter::repeat_with(|| DeployHash::random(rng))
            .take(dependency_count)
            .collect();
        let transfer_args = {
            let mut transfer_args = RuntimeArgs::new();
            let value =
                CLValue::from_t(U512::from(MAX_PAYMENT_AMOUNT)).expect("should create CLValue");
            transfer_args.insert_cl_value(ARG_AMOUNT, value);
            transfer_args
        };
        Deploy::new(
            Timestamp::now(),
            ttl,
            1,
            dependencies,
            chain_name.to_string(),
            ExecutableDeployItem::ModuleBytes {
                module_bytes: Bytes::new(),
                args: RuntimeArgs::new(),
            },
            ExecutableDeployItem::Transfer {
                args: transfer_args,
            },
            &secret_key,
            None,
        )
    }

    #[test]
    fn is_valid() {
        let mut rng = crate::new_rng();
        let mut deploy = create_deploy(&mut rng, DeployConfig::default().max_ttl, 0, "net-1");
        assert_eq!(deploy.is_valid, None, "is valid should initially be None");
        deploy.is_valid().expect("should be valid");
        assert_eq!(deploy.is_valid, Some(Ok(())), "is valid should be true");
    }

    fn check_is_not_valid(mut invalid_deploy: Deploy, expected_error: DeployValidationFailure) {
        assert!(
            invalid_deploy.is_valid.is_none(),
            "is valid should initially be None"
        );
        let actual_error = invalid_deploy.is_valid().unwrap_err();

        // Ignore the `error_msg` field of `InvalidApproval` when comparing to expected error, as
        // this makes the test too fragile.  Otherwise expect the actual error should exactly match
        // the expected error.
        match expected_error {
            DeployValidationFailure::InvalidApproval {
                index: expected_index,
                ..
            } => match actual_error {
                DeployValidationFailure::InvalidApproval {
                    index: actual_index,
                    ..
                } => {
                    assert_eq!(actual_index, expected_index);
                }
                _ => panic!("expected {}, got: {}", expected_error, actual_error),
            },
            _ => {
                assert_eq!(actual_error, expected_error,);
            }
        }

        // The actual error should have been lazily initialized correctly.
        assert_eq!(
            invalid_deploy.is_valid,
            Some(Err(actual_error)),
            "is valid should now be Some"
        );
    }

    #[test]
    fn not_valid_due_to_invalid_body_hash() {
        let mut rng = crate::new_rng();
        let mut deploy = create_deploy(&mut rng, DeployConfig::default().max_ttl, 0, "net-1");

        deploy.session = ExecutableDeployItem::Transfer {
            args: runtime_args! {
                "amount" => 1
            },
        };
        check_is_not_valid(deploy, DeployValidationFailure::InvalidBodyHash);
    }

    #[test]
    fn not_valid_due_to_invalid_deploy_hash() {
        let mut rng = crate::new_rng();
        let mut deploy = create_deploy(&mut rng, DeployConfig::default().max_ttl, 0, "net-1");

        deploy.header.gas_price = 2;
        check_is_not_valid(deploy, DeployValidationFailure::InvalidDeployHash);
    }

    #[test]
    fn not_valid_due_to_invalid_approval() {
        let mut rng = crate::new_rng();
        let mut deploy = create_deploy(&mut rng, DeployConfig::default().max_ttl, 0, "net-1");

        let deploy2 = Deploy::random(&mut rng);

        deploy.approvals.extend(deploy2.approvals);
        check_is_not_valid(
            deploy,
            DeployValidationFailure::InvalidApproval {
                index: 1,
                error_msg: String::new(), // This field is ignored in the check.
            },
        );
    }

    #[test]
    fn is_acceptable() {
        let mut rng = crate::new_rng();
        let chain_name = "net-1";
        let deploy_config = DeployConfig::default();

        let mut deploy = create_deploy(
            &mut rng,
            deploy_config.max_ttl,
            deploy_config.max_dependencies.into(),
            &chain_name,
        );
        deploy
            .is_acceptable(chain_name, &deploy_config)
            .expect("should be acceptable");
    }

    #[test]
    fn not_acceptable_due_to_invalid_chain_name() {
        let mut rng = crate::new_rng();
        let expected_chain_name = "net-1";
        let wrong_chain_name = "net-2".to_string();
        let deploy_config = DeployConfig::default();

        let mut deploy = create_deploy(
            &mut rng,
            deploy_config.max_ttl,
            deploy_config.max_dependencies.into(),
            &wrong_chain_name,
        );

        let expected_error = DeployValidationFailure::InvalidChainName {
            expected: expected_chain_name.to_string(),
            got: wrong_chain_name,
        };

        assert_eq!(
            deploy.is_acceptable(expected_chain_name, &deploy_config),
            Err(expected_error)
        );
        assert!(
            deploy.is_valid.is_none(),
            "deploy should not have run expensive `is_valid` call"
        );
    }

    #[test]
    fn not_acceptable_due_to_excessive_dependencies() {
        let mut rng = crate::new_rng();
        let chain_name = "net-1";
        let deploy_config = DeployConfig::default();

        let dependency_count = usize::from(deploy_config.max_dependencies + 1);

        let mut deploy = create_deploy(
            &mut rng,
            deploy_config.max_ttl,
            dependency_count,
            &chain_name,
        );

        let expected_error = DeployValidationFailure::ExcessiveDependencies {
            max_dependencies: deploy_config.max_dependencies,
            got: dependency_count,
        };

        assert_eq!(
            deploy.is_acceptable(chain_name, &deploy_config),
            Err(expected_error)
        );
        assert!(
            deploy.is_valid.is_none(),
            "deploy should not have run expensive `is_valid` call"
        );
    }

    #[test]
    fn not_acceptable_due_to_excessive_ttl() {
        let mut rng = crate::new_rng();
        let chain_name = "net-1";
        let deploy_config = DeployConfig::default();

        let ttl = deploy_config.max_ttl + TimeDiff::from(Duration::from_secs(1));

        let mut deploy = create_deploy(
            &mut rng,
            ttl,
            deploy_config.max_dependencies.into(),
            &chain_name,
        );

        let expected_error = DeployValidationFailure::ExcessiveTimeToLive {
            max_ttl: deploy_config.max_ttl,
            got: ttl,
        };

        assert_eq!(
            deploy.is_acceptable(chain_name, &deploy_config),
            Err(expected_error)
        );
        assert!(
            deploy.is_valid.is_none(),
            "deploy should not have run expensive `is_valid` call"
        );
    }
}
