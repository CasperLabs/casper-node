use std::{fs::File, path::PathBuf, process, str::FromStr};

use async_trait::async_trait;
use clap::{crate_name, App, Arg, ArgMatches, Shell, SubCommand};

use casper_client::Error;

use crate::{command::ClientCommand, common, Success};

/// This struct defines the order in which the args are shown for this subcommand's help message.
enum DisplayOrder {
    OutputFile,
    Force,
    Shell,
}

/// Handles providing the arg for and retrieval of the output file.
mod output_file {
    use super::*;
    use once_cell::sync::Lazy;

    const ARG_NAME: &str = "output";
    const ARG_NAME_SHORT: &str = "o";
    const ARG_VALUE_NAME: &str = common::ARG_PATH;
    const ARG_HELP: &str =
        "Path to output file. If the path's parent folder doesn't exist, the command will fail. \
        Default path normally requires running the command with sudo";

    static ARG_DEFAULT: Lazy<String> =
        Lazy::new(|| format!("/usr/share/bash-completion/completions/{}", crate_name!()));

    pub(super) fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_NAME_SHORT)
            .required(false)
            .default_value(&*ARG_DEFAULT)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::OutputFile as usize)
    }

    pub(super) fn get(matches: &ArgMatches) -> PathBuf {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
            .into()
    }
}

/// Handles providing the arg for and retrieval of shell type.
mod shell {
    use super::*;

    const ARG_NAME: &str = "shell";
    const ARG_VALUE_NAME: &str = common::ARG_STRING;
    const ARG_DEFAULT: &str = "bash";
    const ARG_HELP: &str = "The type of shell to generate the completion script for";

    pub fn arg() -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .required(false)
            .default_value(ARG_DEFAULT)
            .possible_values(&Shell::variants()[..])
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(DisplayOrder::Shell as usize)
    }

    pub fn get(matches: &ArgMatches) -> Shell {
        Shell::from_str(
            matches
                .value_of(ARG_NAME)
                .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME)),
        )
        .unwrap_or_else(|error| panic!("invalid value for --{}: {}", ARG_NAME, error))
    }
}

pub struct GenerateCompletion {}

#[async_trait]
impl<'a, 'b> ClientCommand<'a, 'b> for GenerateCompletion {
    const NAME: &'static str = "generate-completion";
    const ABOUT: &'static str = "Generates a shell completion script";

    fn build(display_order: usize) -> App<'a, 'b> {
        SubCommand::with_name(Self::NAME)
            .about(Self::ABOUT)
            .display_order(display_order)
            .arg(output_file::arg())
            .arg(common::force::arg(DisplayOrder::Force as usize, true))
            .arg(shell::arg())
    }

    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error> {
        let output_path = output_file::get(matches);
        let force = common::force::get(matches);
        let shell = shell::get(matches);

        if !force && output_path.exists() {
            eprintln!(
                "{} exists. To overwrite, rerun with --{}",
                output_path.display(),
                common::force::ARG_NAME
            );
            process::exit(1);
        }

        let mut output_file = File::create(&output_path).map_err(|error| Error::IoError {
            context: output_path.display().to_string(),
            error,
        })?;
        super::cli().gen_completions_to(crate_name!(), shell, &mut output_file);

        Ok(Success::Output(format!(
            "Wrote completion script for {} to {}",
            shell,
            output_path.display()
        )))
    }
}
