use async_trait::async_trait;
use std::str;

use clap::{App, ArgMatches, SubCommand};

use casper_client::Error;
use casper_types::{AsymmetricType, PublicKey};

use crate::{command::ClientCommand, common, Success};

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    Verbose,
    PublicKey,
}

pub struct GenerateAccountHash {}

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for GenerateAccountHash {
    const NAME: &'static str = "account-address";
    const ABOUT: &'static str = "Generates an account hash from a given public key";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(common::verbose::arg(DisplayOrder::Verbose as usize))
            .arg(common::public_key::arg(DisplayOrder::PublicKey as usize))
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        let hex_public_key = common::public_key::get(matches)?;
        let public_key = PublicKey::from_hex(&hex_public_key).map_err(|error| {
            eprintln!("Can't parse {} as a public key: {}", hex_public_key, error);
            Error::FailedToParseKey
        })?;
        let account_hash = public_key.to_account_hash();
        Ok(Success::Output(account_hash.to_formatted_string()))
    }
}
