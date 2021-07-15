mod generators;

use std::{fs::File, io::BufWriter, path::PathBuf};

use casper_validation::{Fixture, ABI_TEST_FIXTURES};
use clap::Clap;

#[derive(Clap)]
#[clap(version = "1.0")]
struct Opts {
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Clap)]
enum SubCommand {
    Generate(Generate),
}

/// Generates example test fixtures from the code.
///
/// Do not use with day to day development - for example to fix an error in serialization code by
/// replacing the fixture with possibly invalid code.
#[derive(Clap)]
struct Generate {
    /// Path to fixtures directory.
    #[clap(short, long, parse(from_os_str))]
    output: PathBuf,
}

impl Generate {
    fn run(self) -> anyhow::Result<()> {
        let fixtures = generators::make_abi_test_fixtures()?;

        for Fixture::ABI(file_name, fixture) in fixtures {
            let output_path = {
                let mut output_path = self.output.clone();
                output_path.push(ABI_TEST_FIXTURES);
                output_path.push(file_name + ".json");
                output_path
            };

            let file = File::create(&output_path)?;
            let buffered_writer = BufWriter::new(file);
            serde_json::to_writer_pretty(buffered_writer, &fixture)?;
        }

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let opts: Opts = Opts::parse();
    match opts.subcmd {
        SubCommand::Generate(generate) => generate.run()?,
    }
    Ok(())
}
