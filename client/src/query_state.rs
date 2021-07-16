use std::str;

use clap::{App, ArgMatches, SubCommand};

use casper_client::Error;
use casper_node::rpcs::state::GetItem;

use crate::{command::ClientCommand, common, Success};

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    Verbose,
    NodeAddress,
    RpcId,
    StateRootHash,
    Key,
    Path,
}

impl<'a, 'b> ClientCommand<'a, 'b> for GetItem {
    const NAME: &'static str = "query-state";
    const ABOUT: &'static str = "Retrieves a stored value from the network";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::node_address::arg(
                DisplayOrder::NodeAddress as usize,
            ))
            .arg(common::rpc_id::arg(DisplayOrder::RpcId as usize))
            .arg(common::state_root_hash::arg(
                DisplayOrder::StateRootHash as usize,
            ))
            .arg(common::key::arg(DisplayOrder::Key as usize))
            .arg(common::path::arg(DisplayOrder::Path as usize))
    }

    fn run(matches: &ArgMatches<'_>) -> Result<Success, Error> {
        let maybe_rpc_id = common::rpc_id::get(matches);
        let node_address = common::node_address::get(matches);
        let verbosity_level = common::verbose::get(matches);
        let state_root_hash = common::state_root_hash::get(matches);
        let key = common::key::get(matches)?;
        let path = common::path::get(matches);

        casper_client::get_item(
            maybe_rpc_id,
            node_address,
            verbosity_level,
            state_root_hash,
            &key,
            path,
        )
        .map(Success::from)
    }
}
