use std::fs::File;

use async_trait::async_trait;
use jsonrpc_lite::{Id, JsonRpc, Params};
use rand::Rng;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Map, Value};

use casper_execution_engine::core::engine_state::ExecutableDeployItem;
use casper_node::{
    crypto::hash::Digest,
    rpcs::{
        account::{PutDeploy, PutDeployParams},
        chain::{
            BlockIdentifier, GetBlock, GetBlockParams, GetBlockTransfers, GetBlockTransfersParams,
            GetEraInfoBySwitchBlock, GetEraInfoParams, GetStateRootHash, GetStateRootHashParams,
        },
        docs::ListRpcs,
        info::{GetDeploy, GetDeployParams},
        state::{
            GetAccountInfo, GetAccountInfoParams, GetAuctionInfo, GetAuctionInfoParams, GetBalance,
            GetBalanceParams, GetItem, GetItemParams,
        },
        RpcWithOptionalParams, RpcWithParams, RpcWithoutParams, RPC_API_PATH,
    },
    types::{BlockHash, Deploy, DeployHash},
};
use casper_types::{AsymmetricType, Key, PublicKey, URef, U512};

use crate::{
    deploy::{DeployExt, DeployParams, SendDeploy, Transfer},
    error::{Error, Result},
    validation,
};

/// Target for a given transfer.
pub(crate) enum TransferTarget {
    /// Transfer to another account.
    Account(PublicKey),
}

/// Struct representing a single JSON-RPC call to the casper node.
#[derive(Debug)]
pub(crate) struct RpcCall {
    rpc_id: Id,
    node_address: String,
    verbosity_level: u64,
}

/// `RpcCall` encapsulates calls made to the casper node service via JSON-RPC.
impl RpcCall {
    /// Creates a new RPC instance.
    ///
    /// `rpc_id` is used for RPC-ID as required by the JSON-RPC specification, and is returned by
    /// the node in the corresponding response.
    ///
    /// `node_address` identifies the network address of the target node's HTTP server, e.g.
    /// `"http://127.0.0.1:7777"`.
    ///
    /// When `verbosity_level` is `1`, the request will be printed to `stdout` with long string
    /// fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char count
    /// of the field.  When `verbosity_level` is greater than `1`, the request will be printed to
    /// `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
    /// will not be printed to `stdout`.
    pub(crate) fn new(maybe_rpc_id: &str, node_address: &str, verbosity_level: u64) -> Self {
        let rpc_id = if maybe_rpc_id.is_empty() {
            Id::from(rand::thread_rng().gen::<i64>())
        } else if let Ok(i64_id) = maybe_rpc_id.parse::<i64>() {
            Id::from(i64_id)
        } else {
            Id::from(maybe_rpc_id.to_string())
        };

        Self {
            rpc_id,
            node_address: node_address.trim_end_matches('/').to_string(),
            verbosity_level,
        }
    }

    pub(crate) async fn get_deploy(self, deploy_hash: &str) -> Result<JsonRpc> {
        let hash = Digest::from_hex(deploy_hash).map_err(|error| Error::CryptoError {
            context: "deploy_hash",
            error,
        })?;
        let params = GetDeployParams {
            deploy_hash: DeployHash::new(hash),
        };
        GetDeploy::request_with_map_params(self, params).await
    }

    pub(crate) async fn get_item(
        self,
        state_root_hash: &str,
        key: &str,
        path: &str,
    ) -> Result<JsonRpc> {
        let state_root_hash =
            Digest::from_hex(state_root_hash).map_err(|error| Error::CryptoError {
                context: "state_root_hash",
                error,
            })?;

        let key = {
            if let Ok(key) = Key::from_formatted_str(key) {
                key
            } else if let Ok(public_key) = PublicKey::from_hex(key) {
                Key::Account(public_key.to_account_hash())
            } else {
                return Err(Error::FailedToParseKey);
            }
        };

        let path = if path.is_empty() {
            vec![]
        } else {
            path.split('/').map(ToString::to_string).collect()
        };

        let params = GetItemParams {
            state_root_hash,
            key: key.to_formatted_string(),
            path: path.clone(),
        };
        let response = GetItem::request_with_map_params(self, params).await?;
        validation::validate_query_response(&response, &state_root_hash, &key, &path)?;
        Ok(response)
    }

    pub(crate) async fn get_state_root_hash(self, maybe_block_identifier: &str) -> Result<JsonRpc> {
        match Self::block_identifier(maybe_block_identifier)? {
            Some(block_identifier) => {
                let params = GetStateRootHashParams { block_identifier };
                GetStateRootHash::request_with_map_params(self, params).await
            }
            None => GetStateRootHash::request(self).await,
        }
    }

    pub(crate) async fn get_balance(
        self,
        state_root_hash: &str,
        purse_uref: &str,
    ) -> Result<JsonRpc> {
        let state_root_hash =
            Digest::from_hex(state_root_hash).map_err(|error| Error::CryptoError {
                context: "state_root_hash",
                error,
            })?;
        let uref = URef::from_formatted_str(purse_uref)
            .map_err(|error| Error::FailedToParseURef("purse_uref", error))?;
        let key = Key::from(uref);

        let params = GetBalanceParams {
            state_root_hash,
            purse_uref: purse_uref.to_string(),
        };
        let response = GetBalance::request_with_map_params(self, params).await?;
        validation::validate_get_balance_response(&response, &state_root_hash, &key)?;
        Ok(response)
    }

    pub(crate) async fn get_era_info_by_switch_block(
        self,
        maybe_block_identifier: &str,
    ) -> Result<JsonRpc> {
        let response = match Self::block_identifier(maybe_block_identifier)? {
            None => GetEraInfoBySwitchBlock::request(self).await,
            Some(block_identifier) => {
                let params = GetEraInfoParams { block_identifier };
                GetEraInfoBySwitchBlock::request_with_map_params(self, params).await
            }
        }?;
        validation::validate_get_era_info_response(&response)?;
        Ok(response)
    }

    pub(crate) async fn get_auction_info(self, maybe_block_identifier: &str) -> Result<JsonRpc> {
        let response = match Self::block_identifier(maybe_block_identifier)? {
            None => GetAuctionInfo::request(self).await,
            Some(block_identifier) => {
                let params = GetAuctionInfoParams { block_identifier };
                GetAuctionInfo::request_with_map_params(self, params).await
            }
        }?;
        Ok(response)
    }

    pub(crate) async fn list_rpcs(self) -> Result<JsonRpc> {
        ListRpcs::request(self).await
    }

    pub(crate) async fn transfer(
        self,
        amount: U512,
        source_purse: Option<URef>,
        target: TransferTarget,
        transfer_id: u64,
        deploy_params: DeployParams,
        payment: ExecutableDeployItem,
    ) -> Result<JsonRpc> {
        let deploy = Deploy::new_transfer(
            amount,
            source_purse,
            target,
            transfer_id,
            deploy_params,
            payment,
        )?;
        let params = PutDeployParams { deploy };
        Transfer::request_with_map_params(self, params).await
    }

    pub(crate) async fn send_deploy_file(self, input_path: &str) -> Result<JsonRpc> {
        let input = File::open(input_path).map_err(|error| Error::IoError {
            context: format!("unable to read input file '{}'", input_path),
            error,
        })?;
        let deploy = Deploy::read_deploy(input)?;
        let params = PutDeployParams { deploy };
        SendDeploy::request_with_map_params(self, params).await
    }

    pub(crate) async fn put_deploy(self, deploy: Deploy) -> Result<JsonRpc> {
        let params = PutDeployParams { deploy };
        PutDeploy::request_with_map_params(self, params).await
    }

    pub(crate) async fn get_block(self, maybe_block_identifier: &str) -> Result<JsonRpc> {
        let maybe_block_identifier = Self::block_identifier(maybe_block_identifier)?;
        let response = match maybe_block_identifier {
            Some(block_identifier) => {
                let params = GetBlockParams { block_identifier };
                GetBlock::request_with_map_params(self, params).await
            }
            None => GetBlock::request(self).await,
        }?;
        validation::validate_get_block_response(&response, &maybe_block_identifier)?;
        Ok(response)
    }

    pub(crate) async fn get_block_transfers(self, maybe_block_identifier: &str) -> Result<JsonRpc> {
        let maybe_block_identifier = Self::block_identifier(maybe_block_identifier)?;
        let response = match maybe_block_identifier {
            Some(block_identifier) => {
                let params = GetBlockTransfersParams { block_identifier };
                GetBlockTransfers::request_with_map_params(self, params).await
            }
            None => GetBlockTransfers::request(self).await,
        }?;
        Ok(response)
    }

    pub(crate) async fn get_account_info(
        self,
        public_key: &str,
        maybe_block_identifier: &str,
    ) -> Result<JsonRpc> {
        let key = if let Ok(public_key) = PublicKey::from_hex(public_key) {
            public_key
        } else {
            return Err(Error::FailedToParseKey);
        };
        let block_identifier = Self::block_identifier(maybe_block_identifier)?;
        let params = GetAccountInfoParams {
            public_key: key,
            block_identifier,
        };
        GetAccountInfo::request_with_map_params(self, params).await
    }

    fn block_identifier(maybe_block_identifier: &str) -> Result<Option<BlockIdentifier>> {
        if maybe_block_identifier.is_empty() {
            return Ok(None);
        }

        if maybe_block_identifier.len() == (Digest::LENGTH * 2) {
            let hash =
                Digest::from_hex(maybe_block_identifier).map_err(|error| Error::CryptoError {
                    context: "block_identifier",
                    error,
                })?;
            Ok(Some(BlockIdentifier::Hash(BlockHash::new(hash))))
        } else {
            let height = maybe_block_identifier
                .parse()
                .map_err(|error| Error::FailedToParseInt("block_identifier", error))?;
            Ok(Some(BlockIdentifier::Height(height)))
        }
    }

    async fn request(self, method: &str, params: Params) -> Result<JsonRpc> {
        let url = format!("{}/{}", self.node_address, RPC_API_PATH);
        let rpc_req = JsonRpc::request_with_params(self.rpc_id, method, params);

        crate::pretty_print_at_level(&rpc_req, self.verbosity_level);

        let client = Client::new();
        let response = client
            .post(&url)
            .json(&rpc_req)
            .send()
            .await
            .map_err(Error::FailedToGetResponse)?;

        if let Err(error) = response.error_for_status_ref() {
            if self.verbosity_level > 0 {
                println!("Failed Sending {}", error);
            }
            return Err(Error::FailedSending(rpc_req));
        }

        let rpc_response = response.json().await.map_err(Error::FailedToParseResponse);

        if let Err(error) = rpc_response {
            if self.verbosity_level > 0 {
                println!("Failed parsing as a JSON-RPC response: {}", error);
            }
            return Err(error);
        }

        let rpc_response: JsonRpc = rpc_response?;

        if rpc_response.get_result().is_some() {
            if self.verbosity_level > 0 {
                println!("Received successful response:");
            }
            return Ok(rpc_response);
        }

        if let Some(error) = rpc_response.get_error() {
            if self.verbosity_level > 0 {
                println!("Response returned an error");
            }
            return Err(Error::ResponseIsError(error.clone()));
        }

        if self.verbosity_level > 0 {
            println!("Invalid response returned");
        }
        Err(Error::InvalidRpcResponse(rpc_response))
    }
}

/// General purpose client trait for making requests to casper node's HTTP endpoints.
#[async_trait]
pub(crate) trait RpcClient {
    const RPC_METHOD: &'static str;

    /// Calls a casper node's JSON-RPC endpoint.
    async fn request(rpc_call: RpcCall) -> Result<JsonRpc> {
        rpc_call.request(Self::RPC_METHOD, Params::None(())).await
    }

    /// Calls a casper node's JSON-RPC endpoint with parameters.
    async fn request_with_map_params<T: IntoJsonMap + Send>(
        rpc_call: RpcCall,
        params: T,
    ) -> Result<JsonRpc> {
        rpc_call
            .request(Self::RPC_METHOD, Params::from(params.into_json_map()))
            .await
    }
}

impl RpcClient for GetBalance {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetBlock {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetBlockTransfers {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetStateRootHash {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetItem {
    const RPC_METHOD: &'static str = <Self as RpcWithParams>::METHOD;
}

impl RpcClient for GetEraInfoBySwitchBlock {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetAuctionInfo {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for ListRpcs {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetAccountInfo {
    const RPC_METHOD: &'static str = Self::METHOD;
}

pub(crate) trait IntoJsonMap: Serialize {
    fn into_json_map(self) -> Map<String, Value>
    where
        Self: Sized,
    {
        json!(self)
            .as_object()
            .unwrap_or_else(|| panic!("should be a JSON object"))
            .clone()
    }
}

impl IntoJsonMap for PutDeployParams {}
impl IntoJsonMap for GetBlockParams {}
impl IntoJsonMap for GetBlockTransfersParams {}
impl IntoJsonMap for GetStateRootHashParams {}
impl IntoJsonMap for GetDeployParams {}
impl IntoJsonMap for GetBalanceParams {}
impl IntoJsonMap for GetItemParams {}
impl IntoJsonMap for GetEraInfoParams {}
impl IntoJsonMap for ListRpcs {}
impl IntoJsonMap for GetAuctionInfoParams {}
impl IntoJsonMap for GetAccountInfoParams {}
