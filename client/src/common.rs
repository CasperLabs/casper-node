use std::fs;

use clap::{Arg, ArgMatches};

use casper_client::Error;
use casper_types::PublicKey;

pub const ARG_PATH: &str = "PATH";
pub const ARG_HEX_STRING: &str = "HEX STRING";
pub const ARG_STRING: &str = "STRING";
pub const ARG_INTEGER: &str = "INTEGER";

/// Handles the arg for whether verbose output is required or not.
pub mod verbose {
    use super::*;

    pub const ARG_NAME: &str = "verbose";
    const ARG_NAME_SHORT: &str = "v";
    const ARG_HELP: &str =
        "Generates verbose output, e.g. prints the RPC request.  If repeated by using '-vv' then \
        all output will be extra verbose, meaning that large JSON strings will be shown in full";

    pub fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .short(ARG_NAME_SHORT)
            .required(false)
            .multiple(true)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub fn get(matches: &ArgMatches) -> u64 {
        matches.occurrences_of(ARG_NAME)
    }
}

/// Handles providing the arg for and retrieval of the node hostname/IP and port.
pub mod node_address {
    use super::*;

    const ARG_NAME: &str = "node-address";
    const ARG_SHORT: &str = "n";
    const ARG_VALUE_NAME: &str = "HOST:PORT";
    const ARG_DEFAULT: &str = "http://localhost:7777";
    const ARG_HELP: &str = "Hostname or IP and port of node on which HTTP service is running";

    pub fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(true)
            .default_value(ARG_DEFAULT)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
    }
}

/// Handles providing the arg for the RPC ID.
pub mod rpc_id {
    use super::*;

    const ARG_NAME: &str = "id";
    const ARG_VALUE_NAME: &str = "STRING OR INTEGER";
    const ARG_HELP: &str =
        "JSON-RPC identifier, applied to the request and returned in the response. If not \
        provided, a random integer will be assigned";

    pub fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .required(false)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches.value_of(ARG_NAME).unwrap_or_default()
    }
}

/// Handles providing the arg for and retrieval of the secret key.
pub mod secret_key {
    use super::*;

    const ARG_NAME: &str = "secret-key";
    const ARG_SHORT: &str = "k";
    const ARG_VALUE_NAME: &str = super::ARG_PATH;
    const ARG_HELP: &str = "Path to secret key file";

    pub fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(true)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
    }
}

/// Handles the arg for whether to overwrite existing output file(s).
pub mod force {
    use super::*;

    pub const ARG_NAME: &str = "force";
    const ARG_NAME_SHORT: &str = "f";
    const ARG_HELP_SINGULAR: &str =
        "If this flag is passed and the output file already exists, it will be overwritten. \
        Without this flag, if the output file already exists, the command will fail";
    const ARG_HELP_PLURAL: &str =
        "If this flag is passed, any existing output files will be overwritten. Without this flag, \
        if any output file exists, no output files will be generated and the command will fail";

    pub fn arg(order: usize, singular: bool) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_NAME_SHORT)
            .required(false)
            .help(if singular {
                ARG_HELP_SINGULAR
            } else {
                ARG_HELP_PLURAL
            })
            .display_order(order)
    }

    pub fn get(matches: &ArgMatches) -> bool {
        matches.is_present(ARG_NAME)
    }
}

/// Handles providing the arg for and retrieval of the state root hash.
pub mod state_root_hash {
    use super::*;

    const ARG_NAME: &str = "state-root-hash";
    const ARG_SHORT: &str = "s";
    const ARG_VALUE_NAME: &str = super::ARG_HEX_STRING;
    const ARG_HELP: &str = "Hex-encoded hash of the state root";

    pub(crate) fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(true)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub(crate) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME))
    }
}

/// Handles providing the arg for and retrieval of the block hash or block height.
pub mod block_identifier {
    use super::*;

    const ARG_NAME: &str = "block-identifier";
    const ARG_SHORT: &str = "b";
    const ARG_VALUE_NAME: &str = "HEX STRING OR INTEGER";
    const ARG_HELP: &str =
        "Hex-encoded block hash or height of the block. If not given, the last block added to the \
        chain as known at the given node will be used";

    pub(crate) fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(false)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub(crate) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches.value_of(ARG_NAME).unwrap_or_default()
    }
}

/// Handles providing the arg for and retrieval of the public key.
pub mod public_key {
    use casper_node::crypto::AsymmetricKeyExt;
    use casper_types::AsymmetricType;

    use super::*;

    const ARG_NAME: &str = "public-key";
    const ARG_SHORT: &str = "p";
    const ARG_VALUE_NAME: &str = "FORMATTED STRING or PATH";
    const ARG_HELP: &str =
        "This must be a properly formatted public key. The public key may instead be read in from \
        a file, in which case enter the path to the file as the --public-key argument. The file \
        should be one of the two public key files generated via the `keygen` subcommand; \
        \"public_key_hex\" or \"public_key.pem\"";

    pub(crate) fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(true)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub(crate) fn get(matches: &ArgMatches) -> Result<String, Error> {
        let value = matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME));

        // Try to read as a PublicKey PEM file first.
        if let Ok(public_key) = PublicKey::from_file(value) {
            return Ok(public_key.to_hex());
        }

        // Try to read as a hex-encoded PublicKey file next.
        if let Ok(hex_public_key) = fs::read_to_string(value) {
            let _ = PublicKey::from_hex(&hex_public_key).map_err(|error| {
                eprintln!(
                    "Can't parse the contents of {} as a public key: {}",
                    value, error
                );
                Error::FailedToParseKey
            })?;
            return Ok(hex_public_key);
        }

        Ok(value.to_string())
    }
}

/// Handles providing the arg for and retrieval of the key.
pub mod key {
    use casper_node::crypto::AsymmetricKeyExt;
    use casper_types::AsymmetricType;

    use super::*;

    const ARG_NAME: &str = "key";
    const ARG_SHORT: &str = "k";
    const ARG_VALUE_NAME: &str = "FORMATTED STRING or PATH";
    const ARG_HELP: &str =
        "The base key for the query. This must be a properly formatted public key, account hash, \
        contract address hash, URef, transfer hash or deploy-info hash. The format for each \
        respectively is \"<HEX STRING>\", \"account-hash-<HEX STRING>\", \"hash-<HEX STRING>\", \
        \"uref-<HEX STRING>-<THREE DIGIT INTEGER>\", \"transfer-<HEX-STRING>\" and \
        \"deploy-<HEX-STRING>\". The public key may instead be read in from a file, in which case \
        enter the path to the file as the --key argument. The file should be one of the two public \
        key files generated via the `keygen` subcommand; \"public_key_hex\" or \"public_key.pem\"";

    pub(crate) fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(true)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub(crate) fn get(matches: &ArgMatches) -> Result<String, Error> {
        let value = matches
            .value_of(ARG_NAME)
            .unwrap_or_else(|| panic!("should have {} arg", ARG_NAME));

        // Try to read as a PublicKey PEM file first.
        if let Ok(public_key) = PublicKey::from_file(value) {
            return Ok(public_key.to_hex());
        }

        // Try to read as a hex-encoded PublicKey file next.
        if let Ok(hex_public_key) = fs::read_to_string(value) {
            let _ = PublicKey::from_hex(&hex_public_key).map_err(|error| {
                eprintln!(
                    "Can't parse the contents of {} as a public key: {}",
                    value, error
                );
                Error::FailedToParseKey
            })?;
            return Ok(hex_public_key);
        }

        // Just return the value.
        Ok(value.to_string())
    }
}

/// Handles providing the arg for and retrieval of the key.
pub mod path {
    use super::*;

    const ARG_NAME: &str = "query-path";
    const ARG_SHORT: &str = "q";
    const ARG_VALUE_NAME: &str = "PATH/FROM/KEY";
    const ARG_HELP: &str = "The path from the key of the query";

    pub(crate) fn arg(order: usize) -> Arg<'static, 'static> {
        Arg::with_name(ARG_NAME)
            .long(ARG_NAME)
            .short(ARG_SHORT)
            .required(false)
            .value_name(ARG_VALUE_NAME)
            .help(ARG_HELP)
            .display_order(order)
    }

    pub(crate) fn get<'a>(matches: &'a ArgMatches) -> &'a str {
        matches.value_of(ARG_NAME).unwrap_or_default()
    }
}
