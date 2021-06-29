//! Helper struct and function for parsing a chainspec configuration file into its respective domain
//! object.
//!
//! This is necessary because the `network_config` field of the `Chainspec` struct contains a `Vec`
//! of chainspec accounts, but the chainspec toml file contains a path to a further file which
//! contains the accounts' details, rather than the chainspec file containing the accounts' details
//! itself.

use std::{convert::TryFrom, path::Path};

use serde::{Deserialize, Serialize};

use casper_execution_engine::shared::{system_costs::SystemCosts, wasm_config::WasmConfig};
use casper_types::{EraId, ProtocolVersion};

use super::{
    accounts_config::AccountsConfig, global_state_update::GlobalStateUpdateConfig, ActivationPoint,
    Chainspec, CoreConfig, DeployConfig, Error, GlobalStateUpdate, HighwayConfig, NetworkConfig,
    ProtocolConfig,
};
use crate::utils::{self, Loadable};

#[derive(PartialEq, Eq, Serialize, Deserialize, Debug)]
// Disallow unknown fields to ensure config files and command-line overrides contain valid keys.
#[serde(deny_unknown_fields)]
struct TomlNetwork {
    name: String,
    maximum_net_message_size: u32,
}

#[derive(PartialEq, Eq, Serialize, Deserialize, Debug)]
// Disallow unknown fields to ensure config files and command-line overrides contain valid keys.
#[serde(deny_unknown_fields)]
struct TomlProtocol {
    version: ProtocolVersion,
    hard_reset: bool,
    activation_point: ActivationPoint,
    last_emergency_restart: Option<EraId>,
}

/// A chainspec configuration as laid out in the TOML-encoded configuration file.
#[derive(PartialEq, Eq, Serialize, Deserialize, Debug)]
// Disallow unknown fields to ensure config files and command-line overrides contain valid keys.
#[serde(deny_unknown_fields)]
pub(super) struct TomlChainspec {
    protocol: TomlProtocol,
    network: TomlNetwork,
    core: CoreConfig,
    deploys: DeployConfig,
    highway: HighwayConfig,
    wasm: WasmConfig,
    system_costs: SystemCosts,
}

impl From<&Chainspec> for TomlChainspec {
    fn from(chainspec: &Chainspec) -> Self {
        let protocol = TomlProtocol {
            version: chainspec.protocol_config.version,
            hard_reset: chainspec.protocol_config.hard_reset,
            activation_point: chainspec.protocol_config.activation_point,
            last_emergency_restart: chainspec.protocol_config.last_emergency_restart,
        };
        let network = TomlNetwork {
            name: chainspec.network_config.name.clone(),
            maximum_net_message_size: chainspec.network_config.maximum_net_message_size,
        };
        let core = chainspec.core_config;
        let deploys = chainspec.deploy_config;
        let highway = chainspec.highway_config;
        let wasm = chainspec.wasm_config;
        let system_costs = chainspec.system_costs_config;

        TomlChainspec {
            protocol,
            network,
            core,
            deploys,
            highway,
            wasm,
            system_costs,
        }
    }
}

pub(super) fn parse_toml<P: AsRef<Path>>(chainspec_path: P) -> Result<Chainspec, Error> {
    let bytes = utils::read_file(chainspec_path.as_ref()).map_err(Error::LoadChainspec)?;
    let toml_chainspec: TomlChainspec = toml::from_slice(&bytes)?;

    let root = chainspec_path
        .as_ref()
        .parent()
        .unwrap_or_else(|| Path::new(""));

    // accounts.toml must live in the same directory as chainspec.toml.
    let accounts_config = AccountsConfig::from_path(root)?;

    let network_config = NetworkConfig {
        name: toml_chainspec.network.name,
        accounts_config,
        maximum_net_message_size: toml_chainspec.network.maximum_net_message_size,
    };

    let global_state_update = Option::<GlobalStateUpdateConfig>::from_path(root)?
        .map(GlobalStateUpdate::try_from)
        .transpose()?;

    let protocol_config = ProtocolConfig {
        version: toml_chainspec.protocol.version,
        hard_reset: toml_chainspec.protocol.hard_reset,
        activation_point: toml_chainspec.protocol.activation_point,
        global_state_update,
        last_emergency_restart: toml_chainspec.protocol.last_emergency_restart,
    };

    Ok(Chainspec {
        protocol_config,
        network_config,
        core_config: toml_chainspec.core,
        deploy_config: toml_chainspec.deploys,
        highway_config: toml_chainspec.highway,
        wasm_config: toml_chainspec.wasm,
        system_costs_config: toml_chainspec.system_costs,
    })
}
