//! The set of JSON-RPCs which the API server handles.
//!
//! See <https://github.com/CasperLabs/ceps/blob/master/text/0009-client-api.md#rpcs> for info.

pub mod account;
pub mod chain;
pub mod docs;
pub mod info;
pub mod state;

use std::str;

use futures::{future::BoxFuture, TryFutureExt};
use http::Response;
use hyper::Body;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use warp::{
    filters::BoxedFilter,
    reject::{self, Reject},
    Filter,
};
use warp_json_rpc::{filters, Builder};

use casper_types::ProtocolVersion;

use super::{ReactorEventT, RpcRequest};
use crate::effect::EffectBuilder;
use docs::DocExample;

/// The URL path.
pub const RPC_API_PATH: &str = "rpc";

/// Error code returned if the JSON-RPC response indicates failure.
///
/// See <https://www.jsonrpc.org/specification#error_object> for details.
#[repr(i64)]
enum ErrorCode {
    NoSuchDeploy = -32000,
    NoSuchBlock = -32001,
    ParseQueryKey = -32002,
    QueryFailed = -32003,
    QueryFailedToExecute = -32004,
    ParseGetBalanceURef = -32005,
    GetBalanceFailed = -32006,
    GetBalanceFailedToExecute = -32007,
    InvalidDeploy = -32008,
    ParseGetKeysPrefix = -32009,
    GetKeysWithPrefixFailed = -32010,
}

#[derive(Debug)]
pub(super) struct Error(String);

impl Reject for Error {}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Error(error.to_string())
    }
}

/// A JSON-RPC requiring the "params" field to be present.
pub trait RpcWithParams {
    /// The JSON-RPC "method" name.
    const METHOD: &'static str;

    /// The JSON-RPC request's "params" type.
    type RequestParams: Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + DocExample
        + Send
        + 'static;

    /// The JSON-RPC response's "result" type.
    type ResponseResult: Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + DocExample
        + Send
        + 'static;
}

/// A trait for creating a JSON-RPC filter where the request is required to have "params".
pub(super) trait RpcWithParamsExt: RpcWithParams {
    /// Creates the warp filter for this particular RPC.
    fn create_filter<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        api_version: ProtocolVersion,
    ) -> BoxedFilter<(Response<Body>,)> {
        let with_valid_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and(filters::params::<Self::RequestParams>())
            .and_then(
                move |response_builder: Builder, params: Self::RequestParams| {
                    Self::handle_request(effect_builder, response_builder, params, api_version)
                        .map_err(reject::custom)
                },
            );
        let with_invalid_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and(filters::params::<Value>())
            .and_then(
                move |response_builder: Builder, _params: Value| async move {
                    response_builder
                        .error(warp_json_rpc::Error::INVALID_PARAMS)
                        .map_err(|_| reject::reject())
                },
            );
        let with_missing_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and_then(move |response_builder: Builder| async move {
                response_builder
                    .error(warp_json_rpc::Error::INVALID_PARAMS)
                    .map_err(|_| reject::reject())
            });
        with_valid_params
            .or(with_invalid_params)
            .unify()
            .or(with_missing_params)
            .unify()
            .boxed()
    }

    /// Handles the incoming RPC request.
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        params: Self::RequestParams,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>>;
}

/// A JSON-RPC requiring the "params" field to be absent.
pub trait RpcWithoutParams {
    /// The JSON-RPC "method" name.
    const METHOD: &'static str;

    /// The JSON-RPC response's "result" type.
    type ResponseResult: Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + DocExample
        + Send
        + 'static;
}

/// A trait for creating a JSON-RPC filter where the request is not required to have "params".
pub(super) trait RpcWithoutParamsExt: RpcWithoutParams {
    /// Creates the warp filter for this particular RPC.
    fn create_filter<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        api_version: ProtocolVersion,
    ) -> BoxedFilter<(Response<Body>,)> {
        let with_no_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and_then(move |response_builder: Builder| {
                Self::handle_request(effect_builder, response_builder, api_version)
                    .map_err(reject::custom)
            });
        let with_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and(filters::params::<Value>())
            .and_then(
                move |response_builder: Builder, _params: Value| async move {
                    response_builder
                        .error(warp_json_rpc::Error::INVALID_PARAMS)
                        .map_err(|_| reject::reject())
                },
            );
        with_no_params.or(with_params).unify().boxed()
    }

    /// Handles the incoming RPC request.
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>>;
}

/// A JSON-RPC with the "params" field optional.
pub trait RpcWithOptionalParams {
    /// The JSON-RPC "method" name.
    const METHOD: &'static str;

    /// The JSON-RPC request's "params" type.  This will be passed to the handler wrapped in an
    /// `Option`.
    type OptionalRequestParams: Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + DocExample
        + Send
        + 'static;

    /// The JSON-RPC response's "result" type.
    type ResponseResult: Serialize
        + for<'de> Deserialize<'de>
        + JsonSchema
        + DocExample
        + Send
        + 'static;
}

/// A trait for creating a JSON-RPC filter where the request may optionally have "params".
pub(super) trait RpcWithOptionalParamsExt: RpcWithOptionalParams {
    /// Creates the warp filter for this particular RPC.
    fn create_filter<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        api_version: ProtocolVersion,
    ) -> BoxedFilter<(Response<Body>,)> {
        let with_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and(filters::params::<Self::OptionalRequestParams>())
            .and_then(
                move |response_builder: Builder, params: Self::OptionalRequestParams| {
                    Self::handle_request(
                        effect_builder,
                        response_builder,
                        Some(params),
                        api_version,
                    )
                    .map_err(reject::custom)
                },
            );
        let with_invalid_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and(filters::params::<Value>())
            .and_then(
                move |response_builder: Builder, _params: Value| async move {
                    response_builder
                        .error(warp_json_rpc::Error::INVALID_PARAMS)
                        .map_err(|_| reject::reject())
                },
            );
        let without_params = warp::path(RPC_API_PATH)
            .and(filters::json_rpc())
            .and(filters::method(Self::METHOD))
            .and_then(move |response_builder: Builder| {
                Self::handle_request(effect_builder, response_builder, None, api_version)
                    .map_err(reject::custom)
            });
        with_params
            .or(without_params)
            .unify()
            .or(with_invalid_params)
            .unify()
            .boxed()
    }

    /// Handles the incoming RPC request.
    fn handle_request<REv: ReactorEventT>(
        effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        maybe_params: Option<Self::OptionalRequestParams>,
        api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>>;
}

mod common {
    use std::convert::TryFrom;

    use once_cell::sync::Lazy;

    use casper_execution_engine::core::engine_state::{self, QueryResult};
    use casper_types::bytesrepr::ToBytes;

    use super::ErrorCode;
    use crate::types::json_compatibility::StoredValue;

    pub(super) static MERKLE_PROOF: Lazy<String> = Lazy::new(|| {
        String::from(
            "01000000006ef2e0949ac76e55812421f755abe129b6244fe7168b77f47a72536147614625016ef2e0949ac76e\
        55812421f755abe129b6244fe7168b77f47a72536147614625000000003529cde5c621f857f75f3810611eb4af3\
        f998caaa9d4a3413cf799f99c67db0307010000006ef2e0949ac76e55812421f755abe129b6244fe7168b77f47a\
        7253614761462501010102000000006e06000000000074769d28aac597a36a03a932d4b43e4f10bf0403ee5c41d\
        d035102553f5773631200b9e173e8f05361b681513c14e25e3138639eb03232581db7557c9e8dbbc83ce9450022\
        6a9a7fe4f2b7b88d5103a4fc7400f02bf89c860c9ccdd56951a2afe9be0e0267006d820fb5676eb2960e15722f7\
        725f3f8f41030078f8b2e44bf0dc03f71b176d6e800dc5ae9805068c5be6da1a90b2528ee85db0609cc0fb4bd60\
        bbd559f497a98b67f500e1e3e846592f4918234647fca39830b7e1e6ad6f5b7a99b39af823d82ba1873d0000030\
        00000010186ff500f287e9b53f823ae1582b1fa429dfede28015125fd233a31ca04d5012002015cc42669a55467\
        a1fdf49750772bfc1aed59b9b085558eb81510e9b015a7c83b0301e3cf4a34b1db6bfa58808b686cb8fe21ebe0c\
        1bcbcee522649d2b135fe510fe3")
    });

    // Extract the EE `(StoredValue, Vec<TrieMerkleProof<Key, StoredValue>>)` from the result.
    pub(super) fn extract_query_result(
        query_result: Result<QueryResult, engine_state::Error>,
    ) -> Result<(StoredValue, Vec<u8>), (ErrorCode, String)> {
        let (value, proof) = match query_result {
            Ok(QueryResult::Success { value, proofs }) => (value, proofs),
            Ok(query_result) => {
                let error_msg = format!("state query failed: {:?}", query_result);
                return Err((ErrorCode::QueryFailed, error_msg));
            }
            Err(error) => {
                let error_msg = format!("state query failed to execute: {:?}", error);
                return Err((ErrorCode::QueryFailedToExecute, error_msg));
            }
        };

        let value_compat = match StoredValue::try_from(&*value) {
            Ok(value_compat) => value_compat,
            Err(error) => {
                let error_msg = format!("failed to encode stored value: {:?}", error);
                return Err((ErrorCode::QueryFailed, error_msg));
            }
        };

        let proof_bytes = match proof.to_bytes() {
            Ok(proof_bytes) => proof_bytes,
            Err(error) => {
                let error_msg = format!("failed to encode stored value: {:?}", error);
                return Err((ErrorCode::QueryFailed, error_msg));
            }
        };

        Ok((value_compat, proof_bytes))
    }
}
