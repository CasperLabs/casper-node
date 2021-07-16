use async_trait::async_trait;
use clap::{App, Arg, ArgMatches, SubCommand};
use once_cell::sync::Lazy;

use casper_client::{
    keygen::{self, FILES, PUBLIC_KEY_HEX},
    Error,
};

use crate::{command::ClientCommand, common, Success};

static MORE_ABOUT: Lazy<String> = Lazy::new(|| {
    format!(
        "{}. Creates {:?}. \"{}\" contains the hex-encoded key's bytes with the hex-encoded \
        algorithm tag prefixed",
        Keygen::ABOUT,
        FILES,
        PUBLIC_KEY_HEX
    )
});

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    OutputDir,
    Force,
    Algorithm,
}

/// Handles providing the arg for and retrieval of the output directory.
mod output_dir {
    use super::*;

    const ARG_NAME: &str = "output-dir";
    const ARG_VALUE_NAME: &str = common::ARG_PATH;
    const ARG_HELP: &str =
        "Path to output directory where key files will be created. If the path doesn't exist, it \
        will be created. If not set, the current working directory will be used";

    pub(super) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .required(false)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::OutputDir as usize)
    }

    pub(super) fn get(matches: &ArgMatches) -> String {
        matches.value_of(ARG_NAME).unwrap_or(".").to_string()
    }
}

/// Handles providing the arg for and retrieval of the key algorithm.
mod algorithm {
    use super::*;

    const ARG_NAME: &str = "algorithm";
    const ARG_SHORT: &str = "a";
    const ARG_VALUE_NAME: &str = common::ARG_STRING;
    const ARG_HELP: &str = "The type of keys to generate";

    pub fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(false)
            .default_value(keygen::ED25519)
            .possible_value(keygen::ED25519)
            .possible_value(keygen::SECP256K1)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::Algorithm as usize)
    }

    pub fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
    }
}

pub struct Keygen {}

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for Keygen {
    const NAME: &'static str = "keygen";
    const ABOUT: &'static str = "Generates account key files in the given directory";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .long_about(MORE_ABOUT.as_str())
            .display_order(display_order)
            .arg(output_dir::arg())
            .arg(common::force::arg(DisplayOrder::Force as usize, false))
            .arg(algorithm::arg())
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        let output_dir = output_dir::get(matches);
        let algorithm = algorithm::get(matches);
        let force = common::force::get(matches);

        keygen::generate_files(&output_dir, algorithm, force)
            .map(|_| Success::Output(format!("Wrote files to {}", output_dir)))
    }
}
