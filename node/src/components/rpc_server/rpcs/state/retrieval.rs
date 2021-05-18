//! RPCs relating to state retreival for testing purposes.

use futures::{future::BoxFuture, FutureExt};
use http::Response;
use hyper::Body;
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use warp_json_rpc::Builder;

use casper_types::{Key, ProtocolVersion};

use crate::{
    components::rpc_server::{
        rpcs::{docs::DocExample, Error, ErrorCode, RpcWithParams, RpcWithParamsExt},
        ReactorEventT,
    },
    crypto::hash::Digest,
    effect::{requests::RpcRequest, EffectBuilder},
    reactor::QueueKind,
    types::Block,
};
use casper_execution_engine::core::engine_state::query;

static GET_KEYS_WITH_PREFIX_EXAMPLE: GetKeysWithPrefix = GetKeysWithPrefix {};
static GET_KEYS_WITH_PREFIX_PARAMS_EXAMPLE: Lazy<GetKeysWithPrefixParams> =
    Lazy::new(|| GetKeysWithPrefixParams {
        state_root_hash: *Block::doc_example().header().state_root_hash(),
        prefix: String::from("00"),
    });
static GET_KEYS_WITH_PREFIX_RESULT_EXAMPLE: Lazy<GetKeysWithPrefixResult> =
    Lazy::new(|| GetKeysWithPrefixResult::Success { keys: Vec::new() });

/// RPC for returning keys matching a prefix.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct GetKeysWithPrefix {}

impl RpcWithParams for GetKeysWithPrefix {
    const METHOD: &'static str = "state_get_keys_with_prefix";
    type RequestParams = GetKeysWithPrefixParams;
    type ResponseResult = GetKeysWithPrefixResult;
}

impl RpcWithParamsExt for GetKeysWithPrefix {
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        params: Self::RequestParams,
        _api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        async move {
            let state_root_hash: Digest = params.state_root_hash;

            let prefix: Vec<u8> = match hex::decode(params.prefix) {
                Ok(prefix) => prefix,
                Err(error) => {
                    let error_msg = format!("failed to parse prefix: {}", error);
                    return Ok(response_builder.error(warp_json_rpc::Error::custom(
                        ErrorCode::ParseGetKeysPrefix as i64,
                        error_msg,
                    ))?);
                }
            };

            let get_keys_result = effect_builder
                .make_request(
                    |responder| RpcRequest::GetKeysWithPrefix {
                        state_root_hash,
                        prefix,
                        responder,
                    },
                    QueueKind::Api,
                )
                .await;

            let response = match get_keys_result {
                Ok(query::GetKeysWithPrefixResult::Success { keys }) => {
                    let success_response = Self::ResponseResult::Success { keys };
                    response_builder.success(success_response)?
                }
                Ok(query::GetKeysWithPrefixResult::RootNotFound) => {
                    let root_not_found_result = Self::ResponseResult::RootNotFound;
                    response_builder.success(root_not_found_result)?
                }
                Err(error) => {
                    let error_response = warp_json_rpc::Error::custom(
                        ErrorCode::GetKeysWithPrefixFailed as i64,
                        format!("{:?}", error),
                    );
                    response_builder.error(error_response)?
                }
            };
            Ok(response)
        }
        .boxed()
    }
}

impl DocExample for GetKeysWithPrefix {
    fn doc_example() -> &'static Self {
        &GET_KEYS_WITH_PREFIX_EXAMPLE
    }
}

/// Parameters for RPC providing keys by prefix.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct GetKeysWithPrefixParams {
    /// The state root hash at which to gather keys.
    pub state_root_hash: Digest,
    /// The prefix to match against keys.
    pub prefix: String,
}

impl DocExample for GetKeysWithPrefixParams {
    fn doc_example() -> &'static Self {
        &*GET_KEYS_WITH_PREFIX_PARAMS_EXAMPLE
    }
}

/// Result of RPC providing keys by prefix.
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub enum GetKeysWithPrefixResult {
    Success {
        /// Collection of keys matching prefix.
        #[schemars(with = "String", description = "List of keys")]
        keys: Vec<Key>,
    },
    RootNotFound,
}

impl DocExample for GetKeysWithPrefixResult {
    fn doc_example() -> &'static Self {
        &*GET_KEYS_WITH_PREFIX_RESULT_EXAMPLE
    }
}
