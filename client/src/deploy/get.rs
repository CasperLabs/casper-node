use std::str;

use async_trait::async_trait;
use clap::{App, Arg, ArgMatches, SubCommand};

use casper_client::Error;
use casper_node::rpcs::info::GetDeploy;

use crate::{command::ClientCommand, common, Success};

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    Verbose,
    NodeAddress,
    RpcId,
    DeployHash,
}

/// Handles providing the arg for and retrieval of the deploy hash.
mod deploy_hash {
    use super::*;

    const ARG_NAME: &str = "deploy-hash";
    const ARG_VALUE_NAME: &str = "HEX STRING";
    const ARG_HELP: &str = "Hex-encoded deploy hash";

    pub(super) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .required(true)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::DeployHash as usize)
    }

    pub(super) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
    }
}

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for GetDeploy {
    const NAME: &'static str = "get-deploy";
    const ABOUT: &'static str = "Retrieves a deploy from the network";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::node_address::arg(
                DisplayOrder::NodeAddress as usize,
            ))
            .arg(common::rpc_id::arg(DisplayOrder::RpcId as usize))
            .arg(deploy_hash::arg())
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        let maybe_rpc_id = common::rpc_id::get(matches);
        let node_address = common::node_address::get(matches);
        let verbosity_level = common::verbose::get(matches);
        let deploy_hash = deploy_hash::get(matches);

        casper_client::get_deploy(maybe_rpc_id, node_address, verbosity_level, deploy_hash)
            .await
            .map(Success::from)
    }
}
