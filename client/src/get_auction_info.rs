use std::str;

use async_trait::async_trait;
use clap::{App, ArgMatches, SubCommand};

use casper_client::Error;
use casper_node::rpcs::state::GetAuctionInfo;

use crate::{command::ClientCommand, common, Success};

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    Verbose,
    NodeAddress,
    RpcId,
    BlockIdentifier,
}

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for GetAuctionInfo {
    const NAME: &'static str = "get-auction-info";
    const ABOUT: &'static str =
        "Returns the bids and validators as of either a specific block (by height or hash), or the most recently added block";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::node_address::arg(
                DisplayOrder::NodeAddress as usize,
            ))
            .arg(common::rpc_id::arg(DisplayOrder::RpcId as usize))
            .arg(common::block_identifier::arg(
                DisplayOrder::BlockIdentifier as usize,
            ))
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        let maybe_rpc_id = common::rpc_id::get(matches);
        let node_address = common::node_address::get(matches);
        let verbosity_level = common::verbose::get(matches);
        let maybe_block_id = common::block_identifier::get(matches);

        casper_client::get_auction_info(maybe_rpc_id, node_address, verbosity_level, maybe_block_id)
            .await
            .map(Success::from)
    }
}
