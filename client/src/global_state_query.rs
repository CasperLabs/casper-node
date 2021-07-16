use std::str;

use clap::{App, Arg, ArgGroup, ArgMatches, SubCommand};

use casper_client::{Error, GlobalStateStrParams};
use casper_node::rpcs::state::QueryGlobalState;

use crate::{command::ClientCommand, common, Success};

const ARG_HEX_STRING: &str = "HEX STRING";

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    Verbose,
    NodeAddress,
    RpcId,
    BlockHash,
    StateRootHash,
    Key,
    Path,
}

mod state_root_hash {
    use super::*;

    pub(super) const ARG_NAME: &str = "state-root-hash";
    const ARG_SHORT: &str = "s";
    const ARG_VALUE_NAME: &str = ARG_HEX_STRING;
    const ARG_HELP: &str = "Hex-encoded hash of the state root";

    pub(super) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(false)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::StateRootHash as usize)
    }

    pub fn get<'a>(matches: &'a ArgMatches) -> Option<&'a str> {
        matches.value_of(ARG_NAME)
    }
}

mod block_hash {
    use super::*;

    pub(super) const ARG_NAME: &str = "block-hash";
    const ARG_SHORT: &str = "b";
    const ARG_VALUE_NAME: &str = ARG_HEX_STRING;
    const ARG_HELP: &str = "Hex-encoded hash of the block";

    pub(super) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(false)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::BlockHash as usize)
    }

    pub fn get<'a>(matches: &'a ArgMatches) -> Option<&'a str> {
        matches.value_of(ARG_NAME)
    }
}

fn global_state_str_params<'a>(matches: &'a ArgMatches) -> GlobalStateStrParams<'a> {
    if let Some(state_root_hash) = state_root_hash::get(matches) {
        return GlobalStateStrParams {
            is_block_hash: false,
            hash_value: state_root_hash,
        };
    }
    if let Some(block_hash) = block_hash::get(matches) {
        return GlobalStateStrParams {
            is_block_hash: true,
            hash_value: block_hash,
        };
    }
    unreachable!("clap arg groups and parsing should prevent this")
}

impl<'a, 'b> ClientCommand<'a, 'b> for QueryGlobalState {
    const NAME: &'static str = "query-global-state";
    const ABOUT: &'static str =
        "Retrieves a stored value from the network using either the state root hash or block hash";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::node_address::arg(
                DisplayOrder::NodeAddress as usize,
            ))
            .arg(common::rpc_id::arg(DisplayOrder::RpcId as usize))
            .arg(common::key::arg(DisplayOrder::Key as usize))
            .arg(common::path::arg(DisplayOrder::Path as usize))
            .arg(block_hash::arg())
            .arg(state_root_hash::arg())
            .group(
                ArgGroup::with_name("state-identifier")
                    .arg(state_root_hash::ARG_NAME)
                    .arg(block_hash::ARG_NAME)
                    .required(true),
            )
    }

    fn run(matches: &ArgMatches<'_>) -> Result<Success, Error> {
        let maybe_rpc_id = common::rpc_id::get(matches);
        let node_address = common::node_address::get(matches);
        let verbosity_level = common::verbose::get(matches);
        let global_state_str_params = global_state_str_params(matches);
        let key = common::key::get(matches)?;
        let path = common::path::get(matches);

        casper_client::global_state_query(
            maybe_rpc_id,
            node_address,
            verbosity_level,
            global_state_str_params,
            &key,
            path,
        )
        .map(Success::from)
    }
}
