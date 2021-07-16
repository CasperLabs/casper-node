use async_trait::async_trait;
use clap::{App, Arg, ArgMatches, SubCommand};

use casper_client::{DeployStrParams, Error};

use super::creation_common::{self, DisplayOrder};
use crate::{command::ClientCommand, common, Success};

/// Handles providing the arg for and retrieval of the transfer amount.
pub(super) mod amount {
    use super::*;

    const ARG_NAME: &str = "amount";
    const ARG_SHORT: &str = "a";
    const ARG_VALUE_NAME: &str = "512-BIT INTEGER";
    const ARG_HELP: &str = "The number of motes to transfer";

    pub(in crate::deploy) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required_unless(creation_common::show_arg_examples::ARG_NAME)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::TransferAmount as usize)
    }

    pub(in crate::deploy) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
    }
}

/// Handles providing the arg for and retrieval of the target account.
pub(super) mod target_account {
    use super::*;

    pub(in crate::deploy) const ARG_NAME: &str = "target-account";
    const ARG_SHORT: &str = "t";
    const ARG_VALUE_NAME: &str = "HEX STRING";
    const ARG_HELP: &str =
        "Hex-encoded public key of the account from which the main purse will be used as the \
        target";

    // Conflicts with --target-purse, but that's handled via an `ArgGroup` in the subcommand. Don't
    // add a `conflicts_with()` to the arg or the `ArgGroup` fails to work correctly.
    pub(in crate::deploy) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required_unless(creation_common::show_arg_examples::ARG_NAME)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::TransferTargetAccount as usize)
    }

    pub(in crate::deploy) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches.value_of(ARG_NAME).unwrap_or_default()
    }
}

/// Handles providing the arg for and retrieval of the transfer id.
pub(super) mod transfer_id {
    use super::*;

    pub(in crate::deploy) const ARG_NAME: &str = "transfer-id";
    const ARG_SHORT: &str = "i";
    const ARG_VALUE_NAME: &str = "64-BIT INTEGER";
    const ARG_HELP: &str = "User-defined identifier, permanently associated with the transfer";

    pub(in crate::deploy) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required_unless(creation_common::show_arg_examples::ARG_NAME)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::TransferId as usize)
    }

    pub(in crate::deploy) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches.value_of(ARG_NAME).unwrap_or_default()
    }
}

pub struct Transfer {}

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for Transfer {
    const NAME: &'static str = "transfer";
    const ABOUT: &'static str = "Transfers funds between purses";

    fn build(display_order: usize) -> App<'a, 'b> {
        let subcommand = SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::rpc_id::arg(DisplayOrder::RpcId as usize))
            .arg(amount::arg())
            .arg(target_account::arg())
            .arg(transfer_id::arg());
        let subcommand = creation_common::apply_common_payment_options(subcommand);
        creation_common::apply_common_creation_options(subcommand, true)
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        creation_common::show_arg_examples_and_exit_if_required(matches);

        let amount = amount::get(matches);
        let target_account = target_account::get(matches);
        let transfer_id = transfer_id::get(matches);

        let maybe_rpc_id = common::rpc_id::get(matches);
        let node_address = common::node_address::get(matches);
        let verbosity_level = common::verbose::get(matches);

        let secret_key = common::secret_key::get(matches);
        let timestamp = creation_common::timestamp::get(matches);
        let ttl = creation_common::ttl::get(matches);
        let gas_price = creation_common::gas_price::get(matches);
        let dependencies = creation_common::dependencies::get(matches);
        let chain_name = creation_common::chain_name::get(matches);

        let payment_str_params = creation_common::payment_str_params(matches);

        casper_client::transfer(
            maybe_rpc_id,
            node_address,
            verbosity_level,
            amount,
            target_account,
            transfer_id,
            DeployStrParams {
                secret_key,
                timestamp,
                ttl,
                dependencies,
                gas_price,
                chain_name,
            },
            payment_str_params,
        )
        .await
        .map(Success::from)
    }
}
