use std::convert::TryFrom;

use jsonrpc_lite::JsonRpc;
use thiserror::Error;

use casper_execution_engine::{
    core, core::ValidationError, shared::stored_value::StoredValue,
    storage::trie::merkle_proof::TrieMerkleProof,
};
use casper_node::{
    crypto::hash::Digest,
    rpcs::chain::{BlockIdentifier, EraSummary, GetEraInfoResult},
    types::{json_compatibility, Block, BlockValidationError, JsonBlock},
};
use casper_types::{bytesrepr, check_summed_hex, Key, U512};

const GET_ITEM_RESULT_BALANCE_VALUE: &str = "balance_value";
const GET_ITEM_RESULT_STORED_VALUE: &str = "stored_value";
const GET_ITEM_RESULT_MERKLE_PROOF: &str = "merkle_proof";

/// Error that can be returned when validating a block returned from a JSON-RPC method.
#[derive(Error, Debug)]
pub enum ValidateResponseError {
    /// Failed to marshall value.
    #[error("Failed to marshall value {0}")]
    BytesRepr(bytesrepr::Error),

    /// Error from serde.
    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    /// Failed to parse JSON.
    #[error("validate_response failed to parse")]
    ValidateResponseFailedToParse,

    /// Failed to validate Merkle proofs.
    #[error(transparent)]
    ValidationError(#[from] ValidationError),

    /// Failed to validate a block.
    #[error("Block validation error {0}")]
    BlockValidationError(BlockValidationError),

    /// Serialized value not contained in proof.
    #[error("serialized value not contained in proof")]
    SerializedValueNotContainedInProof,

    /// No block in response.
    #[error("no block in response")]
    NoBlockInResponse,

    /// Block hash requested does not correspond to response.
    #[error("block hash requested does not correspond to response")]
    UnexpectedBlockHash,

    /// Block height was not as requested.
    #[error("block height was not as requested")]
    UnexpectedBlockHeight,
}

impl From<bytesrepr::Error> for ValidateResponseError {
    fn from(e: bytesrepr::Error) -> Self {
        ValidateResponseError::BytesRepr(e)
    }
}

impl From<BlockValidationError> for ValidateResponseError {
    fn from(e: BlockValidationError) -> Self {
        ValidateResponseError::BlockValidationError(e)
    }
}

pub(crate) fn validate_get_era_info_response(
    response: &JsonRpc,
) -> Result<(), ValidateResponseError> {
    let value = response
        .get_result()
        .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;

    let result: GetEraInfoResult = serde_json::from_value(value.to_owned())?;

    match result.era_summary {
        Some(EraSummary {
            state_root_hash,
            era_id,
            merkle_proof,
            stored_value,
            ..
        }) => {
            let proof_bytes = check_summed_hex::decode(&merkle_proof)
                .map_err(|_| ValidateResponseError::ValidateResponseFailedToParse)?;
            let proofs: Vec<TrieMerkleProof<Key, StoredValue>> =
                bytesrepr::deserialize(proof_bytes)?;
            let key = Key::EraInfo(era_id);
            let path = &[];

            let proof_value = match stored_value {
                json_compatibility::StoredValue::EraInfo(era_info) => {
                    StoredValue::EraInfo(era_info)
                }
                _ => return Err(ValidateResponseError::ValidateResponseFailedToParse),
            };

            core::validate_query_proof(
                &state_root_hash.to_owned().into(),
                &proofs,
                &key,
                path,
                &proof_value,
            )
            .map_err(Into::into)
        }
        None => Ok(()),
    }
}

pub(crate) fn validate_query_response(
    response: &JsonRpc,
    state_root_hash: &Digest,
    key: &Key,
    path: &[String],
) -> Result<(), ValidateResponseError> {
    let value = response
        .get_result()
        .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;

    let object = value
        .as_object()
        .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;

    let proofs: Vec<TrieMerkleProof<Key, StoredValue>> = {
        let proof = object
            .get(GET_ITEM_RESULT_MERKLE_PROOF)
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        let proof_str = proof
            .as_str()
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        let proof_bytes = check_summed_hex::decode(proof_str)
            .map_err(|_| ValidateResponseError::ValidateResponseFailedToParse)?;
        bytesrepr::deserialize(proof_bytes)?
    };

    let proof_value: &StoredValue = {
        let last_proof = proofs
            .last()
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        last_proof.value()
    };

    // Here we need to validate that JSON `stored_value` is contained in the proof.
    //
    // Possible to deserialize that field into a `StoredValue` and pass below to
    // `validate_query_proof` instead of using this approach?
    {
        let value: json_compatibility::StoredValue = {
            let value = object
                .get(GET_ITEM_RESULT_STORED_VALUE)
                .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
            serde_json::from_value(value.to_owned())?
        };
        match json_compatibility::StoredValue::try_from(proof_value) {
            Ok(json_proof_value) if json_proof_value == value => (),
            _ => return Err(ValidateResponseError::SerializedValueNotContainedInProof),
        }
    }

    core::validate_query_proof(
        &state_root_hash.to_owned().into(),
        &proofs,
        key,
        path,
        proof_value,
    )
    .map_err(Into::into)
}

pub(crate) fn validate_get_balance_response(
    response: &JsonRpc,
    state_root_hash: &Digest,
    key: &Key,
) -> Result<(), ValidateResponseError> {
    let value = response
        .get_result()
        .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;

    let object = value
        .as_object()
        .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;

    let balance_proof: TrieMerkleProof<Key, StoredValue> = {
        let proof = object
            .get(GET_ITEM_RESULT_MERKLE_PROOF)
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        let proof_str = proof
            .as_str()
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        let proof_bytes = check_summed_hex::decode(proof_str)
            .map_err(|_| ValidateResponseError::ValidateResponseFailedToParse)?;
        bytesrepr::deserialize(proof_bytes)?
    };

    let balance: U512 = {
        let value = object
            .get(GET_ITEM_RESULT_BALANCE_VALUE)
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        let value_str = value
            .as_str()
            .ok_or(ValidateResponseError::ValidateResponseFailedToParse)?;
        U512::from_dec_str(value_str)
            .map_err(|_| ValidateResponseError::ValidateResponseFailedToParse)?
    };

    core::validate_balance_proof(
        &state_root_hash.to_owned().into(),
        &balance_proof,
        *key,
        &balance,
    )
    .map_err(Into::into)
}

pub(crate) fn validate_get_block_response(
    response: &JsonRpc,
    maybe_block_identifier: &Option<BlockIdentifier>,
) -> Result<(), ValidateResponseError> {
    let maybe_result = response.get_result();
    let json_block_value = maybe_result
        .and_then(|value| value.get("block"))
        .ok_or(ValidateResponseError::NoBlockInResponse)?;
    let maybe_json_block: Option<JsonBlock> = serde_json::from_value(json_block_value.to_owned())?;
    let json_block = if let Some(json_block) = maybe_json_block {
        json_block
    } else {
        return Ok(());
    };
    let block = Block::from(json_block);
    block.verify()?;
    match maybe_block_identifier {
        Some(BlockIdentifier::Hash(block_hash)) => {
            if block_hash != block.hash() {
                return Err(ValidateResponseError::UnexpectedBlockHash);
            }
        }
        Some(BlockIdentifier::Height(height)) => {
            // More is necessary here to mitigate a MITM attack
            if height != &block.height() {
                return Err(ValidateResponseError::UnexpectedBlockHeight);
            }
        }
        // More is necessary here to mitigate a MITM attack. In this case we would want to validate
        // `block.proofs()` to make sure that 1/3 of the validator weight signed the block, and we
        // would have to know the latest validators through some trustworthy means
        None => (),
    }
    Ok(())
}
