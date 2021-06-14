//! RPCs related to the block chain.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

mod era_summary;

use std::str;

use futures::{future::BoxFuture, FutureExt};
use http::Response;
use hyper::Body;
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::info;
use warp_json_rpc::Builder;

use casper_types::{check_summed_hex, Key, ProtocolVersion, Transfer};

use super::{
    docs::{DocExample, DOCS_EXAMPLE_PROTOCOL_VERSION},
    Error, ErrorCode, ReactorEventT, RpcRequest, RpcWithOptionalParams, RpcWithOptionalParamsExt,
};
use crate::{
    crypto::hash::Digest,
    effect::EffectBuilder,
    reactor::QueueKind,
    rpcs::common::{self},
    types::{Block, BlockHash, BlockSignatures, Item, JsonBlock},
};
pub use era_summary::EraSummary;
use era_summary::ERA_SUMMARY;

static GET_BLOCK_PARAMS: Lazy<GetBlockParams> = Lazy::new(|| GetBlockParams {
    block_identifier: BlockIdentifier::Hash(Block::doc_example().id()),
});
static GET_BLOCK_RESULT: Lazy<GetBlockResult> = Lazy::new(|| GetBlockResult {
    api_version: DOCS_EXAMPLE_PROTOCOL_VERSION,
    block: Some(JsonBlock::doc_example().clone()),
});
static GET_BLOCK_TRANSFERS_PARAMS: Lazy<GetBlockTransfersParams> =
    Lazy::new(|| GetBlockTransfersParams {
        block_identifier: BlockIdentifier::Hash(Block::doc_example().id()),
    });
static GET_BLOCK_TRANSFERS_RESULT: Lazy<GetBlockTransfersResult> =
    Lazy::new(|| GetBlockTransfersResult {
        api_version: DOCS_EXAMPLE_PROTOCOL_VERSION,
        block_hash: Some(Block::doc_example().id()),
        transfers: Some(vec![Transfer::default()]),
    });
static GET_STATE_ROOT_HASH_PARAMS: Lazy<GetStateRootHashParams> =
    Lazy::new(|| GetStateRootHashParams {
        block_identifier: BlockIdentifier::Height(Block::doc_example().header().height()),
    });
static GET_STATE_ROOT_HASH_RESULT: Lazy<GetStateRootHashResult> =
    Lazy::new(|| GetStateRootHashResult {
        api_version: DOCS_EXAMPLE_PROTOCOL_VERSION,
        state_root_hash: Some(*Block::doc_example().header().state_root_hash()),
    });
static GET_ERA_INFO_PARAMS: Lazy<GetEraInfoParams> = Lazy::new(|| GetEraInfoParams {
    block_identifier: BlockIdentifier::Hash(Block::doc_example().id()),
});
static GET_ERA_INFO_RESULT: Lazy<GetEraInfoResult> = Lazy::new(|| GetEraInfoResult {
    api_version: DOCS_EXAMPLE_PROTOCOL_VERSION,
    era_summary: Some(ERA_SUMMARY.clone()),
});

/// Identifier for possible ways to retrieve a block.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum BlockIdentifier {
    /// Identify and retrieve the block with its hash.
    Hash(BlockHash),
    /// Identify and retrieve the block with its height.
    Height(u64),
}

/// Params for "chain_get_block" RPC request.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBlockParams {
    /// The block hash.
    pub block_identifier: BlockIdentifier,
}

impl DocExample for GetBlockParams {
    fn doc_example() -> &'static Self {
        &*GET_BLOCK_PARAMS
    }
}

/// Result for "chain_get_block" RPC response.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBlockResult {
    /// The RPC API version.
    #[schemars(with = "String")]
    pub api_version: ProtocolVersion,
    /// The block, if found.
    pub block: Option<JsonBlock>,
}

impl DocExample for GetBlockResult {
    fn doc_example() -> &'static Self {
        &*GET_BLOCK_RESULT
    }
}

/// "chain_get_block" RPC.
pub struct GetBlock {}

impl RpcWithOptionalParams for GetBlock {
    const METHOD: &'static str = "chain_get_block";
    type OptionalRequestParams = GetBlockParams;
    type ResponseResult = GetBlockResult;
}

impl RpcWithOptionalParamsExt for GetBlock {
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        maybe_params: Option<Self::OptionalRequestParams>,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        async move {
            // Get the block.
            let maybe_block_id = maybe_params.map(|params| params.block_identifier);
            let (block, signatures) =
                match get_block_with_metadata(maybe_block_id, effect_builder).await {
                    Ok(Some((block, signatures))) => (block, signatures),
                    Ok(None) => {
                        let error = warp_json_rpc::Error::custom(
                            ErrorCode::NoSuchBlock as i64,
                            "block not known",
                        );
                        return Ok(response_builder.error(error)?);
                    }
                    Err(error) => return Ok(response_builder.error(error)?),
                };

            let json_block = JsonBlock::new(block, Some(signatures));

            // Return the result.
            let result = Self::ResponseResult {
                api_version,
                block: Some(json_block),
            };
            Ok(response_builder.success(result)?)
        }
        .boxed()
    }
}

/// Params for "chain_get_block_transfers" RPC request.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBlockTransfersParams {
    /// The block hash.
    pub block_identifier: BlockIdentifier,
}

impl DocExample for GetBlockTransfersParams {
    fn doc_example() -> &'static Self {
        &*GET_BLOCK_TRANSFERS_PARAMS
    }
}

/// Result for "chain_get_block_transfers" RPC response.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBlockTransfersResult {
    /// The RPC API version.
    #[schemars(with = "String")]
    pub api_version: ProtocolVersion,
    /// The block hash, if found.
    pub block_hash: Option<BlockHash>,
    /// The block's transfers, if found.
    pub transfers: Option<Vec<Transfer>>,
}

impl GetBlockTransfersResult {
    /// Create an instance of GetBlockTransfersResult.
    pub fn new(
        api_version: ProtocolVersion,
        block_hash: Option<BlockHash>,
        transfers: Option<Vec<Transfer>>,
    ) -> Self {
        GetBlockTransfersResult {
            api_version,
            block_hash,
            transfers,
        }
    }
}

impl DocExample for GetBlockTransfersResult {
    fn doc_example() -> &'static Self {
        &*GET_BLOCK_TRANSFERS_RESULT
    }
}

/// "chain_get_block_transfers" RPC.
pub struct GetBlockTransfers {}

impl RpcWithOptionalParams for GetBlockTransfers {
    const METHOD: &'static str = "chain_get_block_transfers";
    type OptionalRequestParams = GetBlockTransfersParams;
    type ResponseResult = GetBlockTransfersResult;
}

impl RpcWithOptionalParamsExt for GetBlockTransfers {
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        maybe_params: Option<Self::OptionalRequestParams>,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        async move {
            // Get the block.
            let maybe_block_id = maybe_params.map(|params| params.block_identifier);
            let block_hash = match get_block(maybe_block_id, effect_builder).await {
                Ok(Some(block)) => *block.hash(),
                Ok(None) => {
                    return Ok(response_builder.success(Self::ResponseResult::new(
                        api_version,
                        None,
                        None,
                    ))?)
                }
                Err(error) => return Ok(response_builder.error(error)?),
            };

            let transfers = effect_builder
                .make_request(
                    |responder| RpcRequest::GetBlockTransfers {
                        block_hash,
                        responder,
                    },
                    QueueKind::Api,
                )
                .await;

            // Return the result.
            let result = Self::ResponseResult::new(api_version, Some(block_hash), transfers);
            Ok(response_builder.success(result)?)
        }
        .boxed()
    }
}

/// Params for "chain_get_state_root_hash" RPC request.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetStateRootHashParams {
    /// The block hash.
    pub block_identifier: BlockIdentifier,
}

impl DocExample for GetStateRootHashParams {
    fn doc_example() -> &'static Self {
        &*GET_STATE_ROOT_HASH_PARAMS
    }
}

/// Result for "chain_get_state_root_hash" RPC response.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetStateRootHashResult {
    /// The RPC API version.
    #[schemars(with = "String")]
    pub api_version: ProtocolVersion,
    /// Hex-encoded hash of the state root.
    pub state_root_hash: Option<Digest>,
}

impl DocExample for GetStateRootHashResult {
    fn doc_example() -> &'static Self {
        &*GET_STATE_ROOT_HASH_RESULT
    }
}

/// "chain_get_state_root_hash" RPC.
pub struct GetStateRootHash {}

impl RpcWithOptionalParams for GetStateRootHash {
    const METHOD: &'static str = "chain_get_state_root_hash";
    type OptionalRequestParams = GetStateRootHashParams;
    type ResponseResult = GetStateRootHashResult;
}

impl RpcWithOptionalParamsExt for GetStateRootHash {
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        maybe_params: Option<Self::OptionalRequestParams>,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        async move {
            // Get the block.
            let maybe_block_id = maybe_params.map(|params| params.block_identifier);
            let maybe_block = match get_block(maybe_block_id, effect_builder).await {
                Ok(maybe_block) => maybe_block,
                Err(error) => return Ok(response_builder.error(error)?),
            };

            // Return the result.
            let result = Self::ResponseResult {
                api_version,
                state_root_hash: maybe_block.map(|block| *block.state_root_hash()),
            };
            Ok(response_builder.success(result)?)
        }
        .boxed()
    }
}

/// Params for "chain_get_era_info" RPC request.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEraInfoParams {
    /// The block identifier.
    pub block_identifier: BlockIdentifier,
}

impl DocExample for GetEraInfoParams {
    fn doc_example() -> &'static Self {
        &*GET_ERA_INFO_PARAMS
    }
}

/// Result for "chain_get_era_info" RPC response.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEraInfoResult {
    /// The RPC API version.
    #[schemars(with = "String")]
    pub api_version: ProtocolVersion,
    /// The era summary.
    pub era_summary: Option<EraSummary>,
}

impl DocExample for GetEraInfoResult {
    fn doc_example() -> &'static Self {
        &*GET_ERA_INFO_RESULT
    }
}

/// "chain_get_era_info_by_switch_block" RPC
pub struct GetEraInfoBySwitchBlock {}

impl RpcWithOptionalParams for GetEraInfoBySwitchBlock {
    const METHOD: &'static str = "chain_get_era_info_by_switch_block";
    type OptionalRequestParams = GetEraInfoParams;
    type ResponseResult = GetEraInfoResult;
}

impl RpcWithOptionalParamsExt for GetEraInfoBySwitchBlock {
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        maybe_params: Option<Self::OptionalRequestParams>,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        async move {
            // TODO: decide if/how to handle era id
            let maybe_block_id = maybe_params.map(|params| params.block_identifier);
            let maybe_block = match get_block(maybe_block_id, effect_builder).await {
                Ok(maybe_block) => maybe_block,
                Err(error) => return Ok(response_builder.error(error)?),
            };

            let block = match maybe_block {
                Some(block) => block,
                None => {
                    return Ok(response_builder.success(Self::ResponseResult {
                        api_version,
                        era_summary: None,
                    })?)
                }
            };

            let era_id = match block.header().era_end() {
                Some(_) => block.header().era_id(),
                None => {
                    return Ok(response_builder.success(Self::ResponseResult {
                        api_version,
                        era_summary: None,
                    })?)
                }
            };

            let state_root_hash = block.state_root_hash().to_owned();
            let base_key = Key::EraInfo(era_id);
            let path = Vec::new();
            let query_result = effect_builder
                .make_request(
                    |responder| RpcRequest::QueryGlobalState {
                        state_root_hash,
                        base_key,
                        path,
                        responder,
                    },
                    QueueKind::Api,
                )
                .await;

            let (stored_value, proof_bytes) = match common::extract_query_result(query_result) {
                Ok(tuple) => tuple,
                Err((error_code, error_msg)) => {
                    info!("{}", error_msg);
                    return Ok(response_builder
                        .error(warp_json_rpc::Error::custom(error_code as i64, error_msg))?);
                }
            };

            let block_hash = block.hash().to_owned();

            let result = Self::ResponseResult {
                api_version,
                era_summary: Some(EraSummary {
                    block_hash,
                    era_id,
                    stored_value,
                    state_root_hash,
                    merkle_proof: check_summed_hex::encode(&proof_bytes),
                }),
            };

            Ok(response_builder.success(result)?)
        }
        .boxed()
    }
}

async fn get_block<REv: ReactorEventT>(
    maybe_id: Option<BlockIdentifier>,
    effect_builder: EffectBuilder<REv>,
) -> Result<Option<Block>, warp_json_rpc::Error> {
    match get_block_with_metadata(maybe_id, effect_builder).await {
        Ok(Some((block, _))) => Ok(Some(block)),
        Ok(None) => {
            return Err(warp_json_rpc::Error::custom(
                ErrorCode::NoSuchBlock as i64,
                "block not known",
            ))
        }
        Err(error) => Err(error),
    }
}

async fn get_block_with_metadata<REv: ReactorEventT>(
    maybe_id: Option<BlockIdentifier>,
    effect_builder: EffectBuilder<REv>,
) -> Result<Option<(Block, BlockSignatures)>, warp_json_rpc::Error> {
    // Get the block from storage or the latest from the linear chain.
    let getting_specific_block = maybe_id.is_some();
    let maybe_result = effect_builder
        .make_request(
            |responder| RpcRequest::GetBlock {
                maybe_id,
                responder,
            },
            QueueKind::Api,
        )
        .await;

    if maybe_result.is_none() && getting_specific_block {
        info!("failed to get {:?} from storage", maybe_id.unwrap());
        return Err(warp_json_rpc::Error::custom(
            ErrorCode::NoSuchBlock as i64,
            "block not known",
        ));
    }

    Ok(maybe_result)
}
