//! RPCs related to finding information about currently supported RPCs.

// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use futures::{future::BoxFuture, FutureExt};
use http::Response;
use hyper::Body;
use once_cell::sync::Lazy;
use schemars::{
    gen::{SchemaGenerator, SchemaSettings},
    schema::Schema,
    JsonSchema, Map, MapEntry,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use warp_json_rpc::Builder;

use casper_types::ProtocolVersion;

use super::{
    account::PutDeploy,
    chain::{GetBlock, GetBlockTransfers, GetStateRootHash},
    info::{GetDeploy, GetPeers, GetStatus},
    state::{GetAuctionInfo, GetBalance, GetItem},
    Error, ReactorEventT, RpcWithOptionalParams, RpcWithParams, RpcWithoutParams,
    RpcWithoutParamsExt,
};
use crate::{
    effect::EffectBuilder,
    rpcs::{
        chain::GetEraInfoBySwitchBlock,
        state::{GetAccountInfo, QueryGlobalState},
    },
};

pub(crate) const DOCS_EXAMPLE_PROTOCOL_VERSION: ProtocolVersion =
    ProtocolVersion::from_parts(1, 0, 0);

const DEFINITIONS_PATH: &str = "#/components/schemas/";

// As per https://spec.open-rpc.org/#service-discovery-method.
pub(crate) static OPEN_RPC_SCHEMA: Lazy<OpenRpcSchema> = Lazy::new(|| {
    let contact = OpenRpcContactField {
        name: "CasperLabs".to_string(),
        url: "https://casperlabs.io".to_string(),
    };
    let license = OpenRpcLicenseField {
        name: "CasperLabs Open Source License Version 1.0".to_string(),
        url: "https://raw.githubusercontent.com/CasperLabs/casper-node/master/LICENSE".to_string(),
    };
    let info = OpenRpcInfoField {
        version: DOCS_EXAMPLE_PROTOCOL_VERSION.to_string(),
        title: "Client API of Casper Node".to_string(),
        description: "This describes the JSON-RPC 2.0 API of a node on the Casper network."
            .to_string(),
        contact,
        license,
    };

    let server = OpenRpcServerEntry {
        name: "any Casper Network node".to_string(),
        url: "http://IP:PORT/rpc/".to_string(),
    };

    let mut schema = OpenRpcSchema {
        openrpc: "1.0.0-rc1".to_string(),
        info,
        servers: vec![server],
        methods: vec![],
        components: Components {
            schemas: Map::new(),
        },
    };

    schema.push_with_params::<PutDeploy>("receives a Deploy to be executed by the network");
    schema.push_with_params::<GetDeploy>("returns a Deploy from the network");
    schema.push_with_params::<GetAccountInfo>("returns an Account from the network");
    schema.push_with_params::<QueryGlobalState>(
        "a query to global state using either a Block hash or state root hash",
    );
    schema.push_without_params::<GetPeers>("returns a list of peers connected to the node");
    schema.push_without_params::<GetStatus>("returns the current status of the node");
    schema.push_with_optional_params::<GetBlock>("returns a Block from the network");
    schema.push_with_optional_params::<GetBlockTransfers>(
        "returns all transfers for a Block from the network",
    );
    schema.push_with_optional_params::<GetStateRootHash>(
        "returns a state root hash at a given Block",
    );
    schema.push_with_params::<GetItem>("returns a stored value from the network");
    schema.push_with_params::<GetBalance>("returns a purse's balance from the network");
    schema.push_with_optional_params::<GetEraInfoBySwitchBlock>(
        "returns an EraInfo from the network",
    );
    schema.push_with_optional_params::<GetAuctionInfo>(
        "returns the bids and validators as of either a specific block (by height or hash), or the most recently added block",
    );

    schema
});
static LIST_RPCS_RESULT: Lazy<ListRpcsResult> = Lazy::new(|| ListRpcsResult {
    api_version: DOCS_EXAMPLE_PROTOCOL_VERSION,
    name: "OpenRPC Schema".to_string(),
    schema: OPEN_RPC_SCHEMA.clone(),
});

/// A trait used to generate a static hardcoded example of `Self`.
pub trait DocExample {
    /// Generates a hardcoded example of `Self`.
    fn doc_example() -> &'static Self;
}

/// The main schema for the casper node's RPC server, compliant with
/// [the OpenRPC Specification](https://spec.open-rpc.org).
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OpenRpcSchema {
    openrpc: String,
    info: OpenRpcInfoField,
    servers: Vec<OpenRpcServerEntry>,
    methods: Vec<Method>,
    components: Components,
}

impl OpenRpcSchema {
    fn new_generator() -> SchemaGenerator {
        let settings = SchemaSettings::default().with(|settings| {
            settings.definitions_path = DEFINITIONS_PATH.to_string();
        });
        settings.into_generator()
    }

    fn push_with_params<T: RpcWithParams>(&mut self, summary: &str) {
        let mut generator = Self::new_generator();

        let params_schema = T::RequestParams::json_schema(&mut generator);
        let params = Self::make_params(params_schema);

        let result_schema = T::ResponseResult::json_schema(&mut generator);
        let result = ResponseResult {
            name: format!("{}_result", T::METHOD),
            schema: result_schema,
        };

        let examples = vec![Example::from_rpc_with_params::<T>()];

        let method = Method {
            name: T::METHOD.to_string(),
            summary: summary.to_string(),
            params,
            result,
            examples,
        };

        self.methods.push(method);
        self.update_schemas::<T::RequestParams>();
        self.update_schemas::<T::ResponseResult>();
    }

    fn push_without_params<T: RpcWithoutParams>(&mut self, summary: &str) {
        let mut generator = Self::new_generator();

        let result_schema = T::ResponseResult::json_schema(&mut generator);
        let result = ResponseResult {
            name: format!("{}_result", T::METHOD),
            schema: result_schema,
        };

        let examples = vec![Example::from_rpc_without_params::<T>()];

        let method = Method {
            name: T::METHOD.to_string(),
            summary: summary.to_string(),
            params: vec![],
            result,
            examples,
        };

        self.methods.push(method);
        self.update_schemas::<T::ResponseResult>();
    }

    fn push_with_optional_params<T: RpcWithOptionalParams>(&mut self, summary: &str) {
        let mut generator = Self::new_generator();

        let params_schema = T::OptionalRequestParams::json_schema(&mut generator);
        let params = Self::make_params(params_schema);

        let result_schema = T::ResponseResult::json_schema(&mut generator);
        let result = ResponseResult {
            name: format!("{}_result", T::METHOD),
            schema: result_schema,
        };

        let examples = vec![Example::from_rpc_with_optional_params::<T>()];

        // TODO - handle adding a description that the params may be omitted if desired.
        let method = Method {
            name: T::METHOD.to_string(),
            summary: summary.to_string(),
            params,
            result,
            examples,
        };

        self.methods.push(method);
        self.update_schemas::<T::OptionalRequestParams>();
        self.update_schemas::<T::ResponseResult>();
    }

    /// Convert the schema for the params type for T into the OpenRpc-compatible map of name, value
    /// pairs.
    ///
    /// As per the standard, the required params must be sorted before the optional ones.
    fn make_params(schema: Schema) -> Vec<SchemaParam> {
        let schema_object = schema.into_object().object.expect("should be object");
        let mut required_params = schema_object
            .properties
            .iter()
            .filter(|(name, _)| schema_object.required.contains(*name))
            .map(|(name, schema)| SchemaParam {
                name: name.clone(),
                schema: schema.clone(),
                required: true,
            })
            .collect::<Vec<_>>();
        let optional_params = schema_object
            .properties
            .iter()
            .filter(|(name, _)| !schema_object.required.contains(*name))
            .map(|(name, schema)| SchemaParam {
                name: name.clone(),
                schema: schema.clone(),
                required: false,
            })
            .collect::<Vec<_>>();
        required_params.extend(optional_params);
        required_params
    }

    /// Insert the new entries into the #/components/schemas/ map.  Panic if we try to overwrite an
    /// entry with a different value.
    fn update_schemas<S: JsonSchema>(&mut self) {
        let generator = Self::new_generator();
        let mut root_schema = generator.into_root_schema_for::<S>();
        for (key, value) in root_schema.definitions.drain(..) {
            match self.components.schemas.entry(key) {
                MapEntry::Occupied(current_value) => {
                    assert_eq!(
                        current_value.get().clone().into_object().metadata,
                        value.into_object().metadata
                    )
                }
                MapEntry::Vacant(vacant) => {
                    let _ = vacant.insert(value);
                }
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct OpenRpcInfoField {
    version: String,
    title: String,
    description: String,
    contact: OpenRpcContactField,
    license: OpenRpcLicenseField,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct OpenRpcContactField {
    name: String,
    url: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct OpenRpcLicenseField {
    name: String,
    url: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct OpenRpcServerEntry {
    name: String,
    url: String,
}

/// The struct containing the documentation for the RPCs.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Method {
    name: String,
    summary: String,
    params: Vec<SchemaParam>,
    result: ResponseResult,
    examples: Vec<Example>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct SchemaParam {
    name: String,
    schema: Schema,
    required: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct ResponseResult {
    name: String,
    schema: Schema,
}

/// An example pair of request params and response result.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Example {
    name: String,
    params: Vec<ExampleParam>,
    result: ExampleResult,
}

impl Example {
    fn new(method_name: &str, maybe_params_obj: Option<Value>, result_value: Value) -> Self {
        // Break the params struct into an array of param name and value pairs.
        let params = match maybe_params_obj {
            Some(params_obj) => params_obj
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| ExampleParam {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
            None => vec![],
        };

        Example {
            name: format!("{}_example", method_name),
            params,
            result: ExampleResult {
                name: format!("{}_example_result", method_name),
                value: result_value,
            },
        }
    }

    fn from_rpc_with_params<T: RpcWithParams>() -> Self {
        Self::new(
            T::METHOD,
            Some(json!(T::RequestParams::doc_example())),
            json!(T::ResponseResult::doc_example()),
        )
    }

    fn from_rpc_without_params<T: RpcWithoutParams>() -> Self {
        Self::new(T::METHOD, None, json!(T::ResponseResult::doc_example()))
    }

    fn from_rpc_with_optional_params<T: RpcWithOptionalParams>() -> Self {
        Self::new(
            T::METHOD,
            Some(json!(T::OptionalRequestParams::doc_example())),
            json!(T::ResponseResult::doc_example()),
        )
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct ExampleParam {
    name: String,
    value: Value,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct ExampleResult {
    name: String,
    value: Value,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct Components {
    schemas: Map<String, Schema>,
}

/// Result for "rpc.discover" RPC response.
//
// Fields named as per https://spec.open-rpc.org/#service-discovery-method.
#[derive(Clone, Serialize, Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct ListRpcsResult {
    /// The RPC API version.
    #[schemars(with = "String")]
    api_version: ProtocolVersion,
    name: String,
    /// The list of supported RPCs.
    #[schemars(skip)]
    schema: OpenRpcSchema,
}

impl DocExample for ListRpcsResult {
    fn doc_example() -> &'static Self {
        &*LIST_RPCS_RESULT
    }
}

/// "rpc.discover" RPC.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ListRpcs {}

impl RpcWithoutParams for ListRpcs {
    // Named as per https://spec.open-rpc.org/#service-discovery-method.
    const METHOD: &'static str = "rpc.discover";
    type ResponseResult = ListRpcsResult;
}

impl RpcWithoutParamsExt for ListRpcs {
    fn handle_request<REv: ReactorEventT>(
        _effect_builder: EffectBuilder<REv>,
        response_builder: Builder,
        _api_version: ProtocolVersion,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        async move { Ok(response_builder.success(ListRpcsResult::doc_example().clone())?) }.boxed()
    }
}

#[cfg(test)]
mod tests {
    use crate::{types::Chainspec, utils::Loadable};

    use super::*;

    #[test]
    fn check_docs_example_version() {
        let chainspec = Chainspec::from_resources("production");
        assert_eq!(
            DOCS_EXAMPLE_PROTOCOL_VERSION, chainspec.protocol_config.version,
            "DOCS_EXAMPLE_VERSION needs to be updated to match the [protocol.version] in \
            'resources/production/chainspec.toml'"
        );
    }
}
