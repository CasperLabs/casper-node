use std::collections::HashMap;

use casper_execution_engine::{
    core::engine_state::{execution_effect::ExecutionEffect, GetEraValidatorsRequest},
    shared::newtypes::Blake2bHash,
};
use casper_types::{EraId, ExecutionResult, ProtocolVersion};

use crate::types::{Block, DeployHash, DeployHeader};

/// Request for validator weights for a specific era.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorWeightsByEraIdRequest {
    state_hash: Blake2bHash,
    era_id: EraId,
    protocol_version: ProtocolVersion,
}

impl ValidatorWeightsByEraIdRequest {
    /// Constructs a new ValidatorWeightsByEraIdRequest.
    pub fn new(state_hash: Blake2bHash, era_id: EraId, protocol_version: ProtocolVersion) -> Self {
        ValidatorWeightsByEraIdRequest {
            state_hash,
            era_id,
            protocol_version,
        }
    }

    /// Get the state hash.
    pub fn state_hash(&self) -> Blake2bHash {
        self.state_hash
    }

    /// Get the era id.
    pub fn era_id(&self) -> EraId {
        self.era_id
    }

    /// Get the protocol version.
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }
}

impl From<ValidatorWeightsByEraIdRequest> for GetEraValidatorsRequest {
    fn from(input: ValidatorWeightsByEraIdRequest) -> Self {
        GetEraValidatorsRequest::new(input.state_hash, input.protocol_version)
    }
}

/// Request for era validators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EraValidatorsRequest {
    state_hash: Blake2bHash,
    protocol_version: ProtocolVersion,
}

impl EraValidatorsRequest {
    /// Constructs a new EraValidatorsRequest.
    pub fn new(state_hash: Blake2bHash, protocol_version: ProtocolVersion) -> Self {
        EraValidatorsRequest {
            state_hash,
            protocol_version,
        }
    }

    /// Get the state hash.
    pub fn state_hash(&self) -> Blake2bHash {
        self.state_hash
    }

    /// Get the protocol version.
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }
}

impl From<EraValidatorsRequest> for GetEraValidatorsRequest {
    fn from(input: EraValidatorsRequest) -> Self {
        GetEraValidatorsRequest::new(input.state_hash, input.protocol_version)
    }
}

/// A [`Block`] that was the result of execution in the `ContractRuntime` along with any execution
/// effects it may have.
#[derive(Debug)]
pub struct BlockAndExecutionEffects {
    /// The [`Block`] the contract runtime executed.
    pub block: Block,
    /// The results from executing the deploys in the block.
    pub execution_results: HashMap<DeployHash, (DeployHeader, ExecutionResult)>,
    /// An [`ExecutionEffect`] created if an era ended.
    pub maybe_step_execution_effect: Option<ExecutionEffect>,
}
