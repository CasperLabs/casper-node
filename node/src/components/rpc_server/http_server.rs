use std::{convert::Infallible, time::Duration};

use futures::future;
use http::{Response, StatusCode};
use hyper::{
    server::{conn::AddrIncoming, Builder},
    Body,
};
use serde::Serialize;
use tokio::sync::oneshot;
use tower::builder::ServiceBuilder;
use tracing::{info, trace};
use warp::{Filter, Rejection};

use casper_types::ProtocolVersion;

use super::{
    rpcs::{self, RpcWithOptionalParamsExt, RpcWithParamsExt, RpcWithoutParamsExt, RPC_API_PATH},
    ReactorEventT,
};
use crate::effect::EffectBuilder;

// This is a workaround for not being able to create a `warp_json_rpc::Response` without a
// `warp_json_rpc::Builder`.
fn new_error_response(error: warp_json_rpc::Error) -> Response<Body> {
    #[derive(Serialize)]
    struct JsonRpcErrorResponse {
        jsonrpc: String,
        id: Option<()>,
        error: warp_json_rpc::Error,
    }

    let json_response = JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        id: None,
        error,
    };

    let body = Body::from(serde_json::to_vec(&json_response).unwrap());
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(body)
        .unwrap()
}

/// Run the JSON-RPC server.
pub(super) async fn run<REv: ReactorEventT>(
    builder: Builder<AddrIncoming>,
    effect_builder: EffectBuilder<REv>,
    api_version: ProtocolVersion,
    qps_limit: u64,
) {
    // RPC filters.
    let rpc_put_deploy = rpcs::account::PutDeploy::create_filter(effect_builder, api_version);
    let rpc_get_block = rpcs::chain::GetBlock::create_filter(effect_builder, api_version);
    let rpc_get_block_transfers =
        rpcs::chain::GetBlockTransfers::create_filter(effect_builder, api_version);
    let rpc_get_state_root_hash =
        rpcs::chain::GetStateRootHash::create_filter(effect_builder, api_version);
    let rpc_get_item = rpcs::state::GetItem::create_filter(effect_builder, api_version);
    let rpc_query_global_state =
        rpcs::state::QueryGlobalState::create_filter(effect_builder, api_version);
    let rpc_get_balance = rpcs::state::GetBalance::create_filter(effect_builder, api_version);
    let rpc_get_account_info =
        rpcs::state::GetAccountInfo::create_filter(effect_builder, api_version);
    let rpc_get_deploy = rpcs::info::GetDeploy::create_filter(effect_builder, api_version);
    let rpc_get_peers = rpcs::info::GetPeers::create_filter(effect_builder, api_version);
    let rpc_get_status = rpcs::info::GetStatus::create_filter(effect_builder, api_version);
    let rpc_get_era_info =
        rpcs::chain::GetEraInfoBySwitchBlock::create_filter(effect_builder, api_version);
    let rpc_get_auction_info =
        rpcs::state::GetAuctionInfo::create_filter(effect_builder, api_version);
    let rpc_get_rpcs = rpcs::docs::ListRpcs::create_filter(effect_builder, api_version);

    // Catch requests where the method is not one we handle.
    let unknown_method = warp::path(RPC_API_PATH)
        .and(warp_json_rpc::filters::json_rpc())
        .and_then(move |response_builder: warp_json_rpc::Builder| async move {
            response_builder
                .error(warp_json_rpc::Error::METHOD_NOT_FOUND)
                .map_err(|_| warp::reject())
        });

    // Catch requests which don't parse as JSON.
    let parse_failure = warp::path(RPC_API_PATH).and_then(move || async move {
        let error_response = new_error_response(warp_json_rpc::Error::PARSE_ERROR);
        Ok::<_, Rejection>(error_response)
    });

    // TODO - we can't catch cases where we should return `warp_json_rpc::Error::INVALID_REQUEST`
    //        (i.e. where the request is JSON, but not valid JSON-RPC).  This will require an
    //        update to or move away from warp_json_rpc.
    let service = warp_json_rpc::service(
        rpc_put_deploy
            .or(rpc_get_block)
            .or(rpc_get_block_transfers)
            .or(rpc_get_state_root_hash)
            .or(rpc_get_item)
            .or(rpc_query_global_state)
            .or(rpc_get_balance)
            .or(rpc_get_deploy)
            .or(rpc_get_peers)
            .or(rpc_get_status)
            .or(rpc_get_era_info)
            .or(rpc_get_auction_info)
            .or(rpc_get_account_info)
            .or(rpc_get_rpcs)
            .or(unknown_method)
            .or(parse_failure),
    );

    // Start the server, passing a oneshot receiver to allow the server to be shut down gracefully.
    let make_svc =
        hyper::service::make_service_fn(move |_| future::ok::<_, Infallible>(service.clone()));

    let make_svc = ServiceBuilder::new()
        .rate_limit(qps_limit, Duration::from_secs(1))
        .service(make_svc);

    let (shutdown_sender, shutdown_receiver) = oneshot::channel::<()>();

    let server = builder.serve(make_svc);
    info!(address = %server.local_addr(), "started JSON-RPC server");

    let server_with_shutdown = server.with_graceful_shutdown(async {
        shutdown_receiver.await.ok();
    });

    let server_joiner = tokio::spawn(server_with_shutdown);

    let _ = server_joiner.await;

    // Shut down the server.
    let _ = shutdown_sender.send(());

    trace!("JSON-RPC server stopped");
}
