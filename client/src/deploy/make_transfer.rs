use async_trait::async_trait;
use clap::{App, ArgMatches, SubCommand};

use casper_client::{DeployStrParams, Error};

use super::{creation_common, transfer};
use crate::{command::ClientCommand, common, Success};

pub struct MakeTransfer;

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for MakeTransfer {
    const NAME: &'static str = "make-transfer";
    const ABOUT: &'static str =
        "Creates a transfer deploy and outputs it to a file or stdout. As a file, the deploy can \
        subsequently be signed by other parties using the 'sign-deploy' subcommand and then sent \
        to the network for execution using the 'send-deploy' subcommand";

    fn build(display_order: usize) -> App<'a, 'b> {
        let subcommand = SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(creation_common::output::arg())
            .arg(transfer::amount::arg())
            .arg(transfer::target_account::arg())
            .arg(transfer::transfer_id::arg())
            .arg(common::force::arg(
                creation_common::DisplayOrder::Force as usize,
                true,
            ));
        let subcommand = creation_common::apply_common_payment_options(subcommand);
        creation_common::apply_common_creation_options(subcommand, false)
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        creation_common::show_arg_examples_and_exit_if_required(matches);

        let amount = transfer::amount::get(matches);
        let target_account = transfer::target_account::get(matches);
        let transfer_id = transfer::transfer_id::get(matches);

        let secret_key = common::secret_key::get(matches);
        let timestamp = creation_common::timestamp::get(matches);
        let ttl = creation_common::ttl::get(matches);
        let gas_price = creation_common::gas_price::get(matches);
        let dependencies = creation_common::dependencies::get(matches);
        let chain_name = creation_common::chain_name::get(matches);

        let payment_str_params = creation_common::payment_str_params(matches);

        let maybe_output_path = creation_common::output::get(matches).unwrap_or_default();
        let force = common::force::get(matches);

        casper_client::make_transfer(
            maybe_output_path,
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
            force,
        )
        .map(|_| {
            Success::Output(if maybe_output_path.is_empty() {
                String::new()
            } else {
                format!("Wrote the transfer deploy to {}", maybe_output_path)
            })
        })
    }
}
