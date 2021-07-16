use async_trait::async_trait;
use clap::{App, ArgMatches};
use jsonrpc_lite::JsonRpc;

use casper_client::Error;

/// The result of a successful execution of a given client command.
pub enum Success {
    /// The success response to a JSON-RPC request.
    Response(JsonRpc),
    /// The output which should be presented to the user for non-RPC client commands.
    Output(String),
}

impl From<JsonRpc> for Success {
    fn from(response: JsonRpc) -> Self {
        Success::Response(response)
    }
}

#[async_trait]
pub trait ClientCommand<'a, 'b> {
    const NAME: &'static str;
    const ABOUT: &'static str;
    /// Constructs the clap `SubCommand` and returns the clap `App`.
    fn build(display_order: usize) -> App<'a, 'b>;

    /// Parses the arg matches and runs the subcommand.
    async fn run(matches: &ArgMatches<'a>) -> Result<Success, Error>;
}
