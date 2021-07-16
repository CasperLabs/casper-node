use async_trait::async_trait;
use clap::{App, ArgMatches, SubCommand};

use casper_client::{DeployStrParams, Error};
use casper_node::rpcs::account::PutDeploy;

use super::creation_common::{self, DisplayOrder};
use crate::{command::ClientCommand, common, Success};

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for PutDeploy {
    const NAME: &'static str = "put-deploy";
    const ABOUT: &'static str = "Creates a deploy and sends it to the network for execution";

    fn build(display_order: usize) -> App<'a, 'b> {
        let subcommand = SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::rpc_id::arg(DisplayOrder::RpcId as usize));
        let subcommand = creation_common::apply_common_session_options(subcommand);
        let subcommand = creation_common::apply_common_payment_options(subcommand);
        creation_common::apply_common_creation_options(subcommand, true)
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        creation_common::show_arg_examples_and_exit_if_required(matches);

        let maybe_rpc_id = common::rpc_id::get(matches);
        let node_address = common::node_address::get(matches);
        let verbosity_level = common::verbose::get(matches);

        let secret_key = common::secret_key::get(matches);
        let timestamp = creation_common::timestamp::get(matches);
        let ttl = creation_common::ttl::get(matches);
        let gas_price = creation_common::gas_price::get(matches);
        let dependencies = creation_common::dependencies::get(matches);
        let chain_name = creation_common::chain_name::get(matches);

        let session_str_params = creation_common::session_str_params(matches);
        let payment_str_params = creation_common::payment_str_params(matches);

        casper_client::put_deploy(
            maybe_rpc_id,
            node_address,
            verbosity_level,
            DeployStrParams {
                secret_key,
                timestamp,
                ttl,
                dependencies,
                gas_price,
                chain_name,
            },
            session_str_params,
            payment_str_params,
        )
        .await
        .map(Success::from)
    }
}
