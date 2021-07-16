//! # Casper node client library
#![doc(
    html_root_url = "https://docs.rs/casper-client/0.1.0",
    html_favicon_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Favicon_RGB_50px.png",
    html_logo_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Symbol_RGB.png",
    test(attr(forbid(warnings)))
)]
#![warn(
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_qualifications
)]

mod cl_type;
mod deploy;
mod error;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod keygen;
mod parsing;
mod rpc;
mod validation;

use std::{convert::TryInto, fs, io::Cursor};

use jsonrpc_lite::JsonRpc;
use serde::Serialize;

use casper_execution_engine::core::engine_state::ExecutableDeployItem;
use casper_node::types::Deploy;
use casper_types::{UIntParseError, U512};

pub use cl_type::help;
pub use deploy::ListDeploysResult;
use deploy::{DeployExt, DeployParams, OutputKind};
pub use error::Error;
use error::Result;
use rpc::{RpcCall, TransferTarget};
pub use validation::ValidateResponseError;

/// Creates a `Deploy` and sends it to the network for execution.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `deploy_params` contains deploy-related options for this `Deploy`. See
///   [`DeployStrParams`](struct.DeployStrParams.html) for more details.
/// * `session_params` contains session-related options for this `Deploy`. See
///   [`SessionStrParams`](struct.SessionStrParams.html) for more details.
/// * `payment_params` contains payment-related options for this `Deploy`. See
///   [`PaymentStrParams`](struct.PaymentStrParams.html) for more details.
pub async fn put_deploy(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    deploy_params: DeployStrParams<'_>,
    session_params: SessionStrParams<'_>,
    payment_params: PaymentStrParams<'_>,
) -> Result<JsonRpc> {
    let deploy = Deploy::with_payment_and_session(
        deploy_params.try_into()?,
        payment_params.try_into()?,
        session_params.try_into()?,
    )?;
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .put_deploy(deploy)
        .await
}

/// Creates a `Deploy` and outputs it to a file or stdout.
///
/// As a file, the `Deploy` can subsequently be signed by other parties using
/// [`sign_deploy_file()`](fn.sign_deploy_file.html) and then sent to the network for execution
/// using [`send_deploy_file()`](fn.send_deploy_file.html).
///
/// * `maybe_output_path` specifies the output file, or if empty, will print it to `stdout`.
/// * `deploy_params` contains deploy-related options for this `Deploy`. See
///   [`DeployStrParams`](struct.DeployStrParams.html) for more details.
/// * `session_params` contains session-related options for this `Deploy`. See
///   [`SessionStrParams`](struct.SessionStrParams.html) for more details.
/// * `payment_params` contains payment-related options for this `Deploy`. See
///   [`PaymentStrParams`](struct.PaymentStrParams.html) for more details.
/// * If `force` is true, and a file exists at `maybe_output_path`, it will be overwritten. If
///   `force` is false and a file exists at `maybe_output_path`,
///   [`Error::FileAlreadyExists`](enum.Error.html#variant.FileAlreadyExists) is returned and a file
///   will not be written.
pub fn make_deploy(
    maybe_output_path: &str,
    deploy_params: DeployStrParams<'_>,
    session_params: SessionStrParams<'_>,
    payment_params: PaymentStrParams<'_>,
    force: bool,
) -> Result<()> {
    let output = if maybe_output_path.is_empty() {
        OutputKind::Stdout
    } else {
        OutputKind::file(maybe_output_path, force)
    };

    Deploy::with_payment_and_session(
        deploy_params.try_into()?,
        payment_params.try_into()?,
        session_params.try_into()?,
    )?
    .write_deploy(output.get()?)?;

    output.commit()
}

/// Reads a previously-saved `Deploy` from a file, cryptographically signs it, and outputs it to a
/// file or stdout.
///
/// * `input_path` specifies the path to the previously-saved `Deploy` file.
/// * `secret_key` specifies the path to the secret key with which to sign the `Deploy`.
/// * `maybe_output_path` specifies the output file, or if empty, will print it to `stdout`.
/// * If `force` is true, and a file exists at `maybe_output_path`, it will be overwritten. If
///   `force` is false and a file exists at `maybe_output_path`,
///   [`Error::FileAlreadyExists`](enum.Error.html#variant.FileAlreadyExists) is returned and a file
///   will not be written.
pub fn sign_deploy_file(
    input_path: &str,
    secret_key: &str,
    maybe_output_path: &str,
    force: bool,
) -> Result<()> {
    let secret_key = parsing::secret_key(secret_key)?;

    let input = fs::read(input_path).map_err(|error| Error::IoError {
        context: format!("unable to read deploy file at '{}'", input_path),
        error,
    })?;

    let output = if maybe_output_path.is_empty() {
        OutputKind::Stdout
    } else {
        OutputKind::file(maybe_output_path, force)
    };

    Deploy::sign_and_write_deploy(Cursor::new(input), secret_key, output.get()?)?;

    output.commit()
}

/// Reads a previously-saved `Deploy` from a file and sends it to the network for execution.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `input_path` specifies the path to the previously-saved `Deploy` file.
pub async fn send_deploy_file(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    input_path: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .send_deploy_file(input_path)
        .await
}

/// Transfers funds between purses.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `amount` is a string to be parsed as a `U512` specifying the amount to be transferred.
/// * `target_account` is the account `PublicKey` into which the funds will be transferred,
///   formatted as a hex-encoded string. The account's main purse will receive the funds.
/// * `transfer_id` is a string to be parsed as a `u64` representing a user-defined identifier which
///   will be permanently associated with the transfer.
/// * `deploy_params` contains deploy-related options for this `Deploy`. See
///   [`DeployStrParams`](struct.DeployStrParams.html) for more details.
/// * `payment_params` contains payment-related options for this `Deploy`. See
///   [`PaymentStrParams`](struct.PaymentStrParams.html) for more details.
#[allow(clippy::too_many_arguments)]
pub async fn transfer(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    amount: &str,
    target_account: &str,
    transfer_id: &str,
    deploy_params: DeployStrParams<'_>,
    payment_params: PaymentStrParams<'_>,
) -> Result<JsonRpc> {
    let amount = U512::from_dec_str(amount)
        .map_err(|err| Error::FailedToParseUint("amount", UIntParseError::FromDecStr(err)))?;
    let source_purse = None;
    let target = parsing::get_transfer_target(target_account)?;
    let transfer_id = parsing::transfer_id(transfer_id)?;

    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .transfer(
            amount,
            source_purse,
            target,
            transfer_id,
            deploy_params.try_into()?,
            payment_params.try_into()?,
        )
        .await
}

/// Creates a transfer `Deploy` and outputs it to a file or stdout.
///
/// As a file, the transfer `Deploy` can subsequently be signed by other parties using
/// [`sign_deploy_file()`](fn.sign_deploy_file.html) and then sent to the network for execution
/// using [`send_deploy_file()`](fn.send_deploy_file.html).
///
/// * `maybe_output_path` specifies the output file, or if empty, will print it to `stdout`.
/// * `amount` is a string to be parsed as a `U512` specifying the amount to be transferred.
/// * `target_account` is the account `PublicKey` into which the funds will be transferred,
///   formatted as a hex-encoded string. The account's main purse will receive the funds.
/// * `transfer_id` is a string to be parsed as a `u64` representing a user-defined identifier which
///   will be permanently associated with the transfer.
/// * `deploy_params` contains deploy-related options for this `Deploy`. See
///   [`DeployStrParams`](struct.DeployStrParams.html) for more details.
/// * `payment_params` contains payment-related options for this `Deploy`. See
///   [`PaymentStrParams`](struct.PaymentStrParams.html) for more details.
/// * If `force` is true, and a file exists at `maybe_output_path`, it will be overwritten. If
///   `force` is false and a file exists at `maybe_output_path`,
///   [`Error::FileAlreadyExists`](enum.Error.html#variant.FileAlreadyExists) is returned and a file
///   will not be written.
pub fn make_transfer(
    maybe_output_path: &str,
    amount: &str,
    target_account: &str,
    transfer_id: &str,
    deploy_params: DeployStrParams<'_>,
    payment_params: PaymentStrParams<'_>,
    force: bool,
) -> Result<()> {
    let amount = U512::from_dec_str(amount)
        .map_err(|err| Error::FailedToParseUint("amount", UIntParseError::FromDecStr(err)))?;
    let source_purse = None;
    let target = parsing::get_transfer_target(target_account)?;
    let transfer_id = parsing::transfer_id(transfer_id)?;

    let output = if maybe_output_path.is_empty() {
        OutputKind::Stdout
    } else {
        OutputKind::file(maybe_output_path, force)
    };

    Deploy::new_transfer(
        amount,
        source_purse,
        target,
        transfer_id,
        deploy_params.try_into()?,
        payment_params.try_into()?,
    )?
    .write_deploy(output.get()?)?;

    output.commit()
}

/// Retrieves a `Deploy` from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `deploy_hash` must be a hex-encoded, 32-byte hash digest.
pub async fn get_deploy(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    deploy_hash: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_deploy(deploy_hash)
        .await
}

/// Retrieves a `Block` from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `maybe_block_id` must be a hex-encoded, 32-byte hash digest or a `u64` representing the
///   `Block` height or empty. If empty, the latest `Block` will be retrieved.
pub async fn get_block(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    maybe_block_id: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_block(maybe_block_id)
        .await
}

/// Retrieves all `Transfer` items for a `Block` from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `maybe_block_id` must be a hex-encoded, 32-byte hash digest or a `u64` representing the
///   `Block` height or empty. If empty, the latest `Block` transfers will be retrieved.
pub async fn get_block_transfers(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    maybe_block_id: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_block_transfers(maybe_block_id)
        .await
}

/// Retrieves a state root hash at a given `Block`.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `maybe_block_id` must be a hex-encoded, 32-byte hash digest or a `u64` representing the
///   `Block` height or empty. If empty, the latest `Block` will be used.
pub async fn get_state_root_hash(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    maybe_block_id: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_state_root_hash(maybe_block_id)
        .await
}

/// Retrieves a stored value from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `state_root_hash` must be a hex-encoded, 32-byte hash digest.
/// * `key` must be a formatted [`PublicKey`](https://docs.rs/casper-node/latest/casper-node/crypto/asymmetric_key/enum.PublicKey.html)
///   or [`Key`](https://docs.rs/casper-types/latest/casper-types/enum.PublicKey.html). This will
///   take one of the following forms:
/// ```text
/// 01c9e33693951aaac23c49bee44ad6f863eedcd38c084a3a8f11237716a3df9c2c           # PublicKey
/// account-hash-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20  # Key::Account
/// hash-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20        # Key::Hash
/// uref-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20-007    # Key::URef
/// transfer-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20    # Key::Transfer
/// deploy-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20      # Key::DeployInfo
/// ```
/// * `path` is comprised of components starting from the `key`, separated by `/`s.
pub async fn get_item(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    state_root_hash: &str,
    key: &str,
    path: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_item(state_root_hash, key, path)
        .await
}

/// Retrieves a purse's balance from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `state_root_hash` must be a hex-encoded, 32-byte hash digest.
/// * `purse` is a URef, formatted as e.g.
/// ```text
/// uref-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20-007
/// ```
pub async fn get_balance(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    state_root_hash: &str,
    purse: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_balance(state_root_hash, purse)
        .await
}

/// Retrieves era information from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `maybe_block_id` must be a hex-encoded, 32-byte hash digest or a `u64` representing the
///   `Block` height or empty. If empty, era information from the latest block will be returned if
///   available.
pub async fn get_era_info_by_switch_block(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    maybe_block_id: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_era_info_by_switch_block(maybe_block_id)
        .await
}

/// Retrieves the bids and validators as of the most recently added `Block`.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `maybe_block_id` must be a hex-encoded, 32-byte hash digest or a `u64` representing the
///   `Block` height or empty. If empty, era information from the latest block will be returned if
///   available.
pub async fn get_auction_info(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    maybe_block_id: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_auction_info(maybe_block_id)
        .await
}

/// Retrieves an Account from the network.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
/// * `public_key` the public key associated with the `Account`
/// * `maybe_block_id` must be a hex-encoded, 32-byte hash digest or a `u64` representing the
///   `Block` height or empty. If empty, the latest `Block` will be retrieved.
pub async fn get_account_info(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
    public_key: &str,
    maybe_block_id: &str,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .get_account_info(public_key, maybe_block_id)
        .await
}

/// Retrieves information and examples for all currently supported RPCs.
///
/// * `maybe_rpc_id` is the JSON-RPC identifier, applied to the request and returned in the
///   response. If it can be parsed as an `i64` it will be used as a JSON integer. If empty, a
///   random `i64` will be assigned. Otherwise the provided string will be used verbatim.
/// * `node_address` is the hostname or IP and port of the node on which the HTTP service is
///   running, e.g. `"http://127.0.0.1:7777"`.
/// * When `verbosity_level` is `1`, the JSON-RPC request will be printed to `stdout` with long
///   string fields (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char
///   count of the field.  When `verbosity_level` is greater than `1`, the request will be printed
///   to `stdout` with no abbreviation of long fields.  When `verbosity_level` is `0`, the request
///   will not be printed to `stdout`.
pub async fn list_rpcs(
    maybe_rpc_id: &str,
    node_address: &str,
    verbosity_level: u64,
) -> Result<JsonRpc> {
    RpcCall::new(maybe_rpc_id, node_address, verbosity_level)
        .list_rpcs()
        .await
}

/// Container for `Deploy` construction options.
#[derive(Default, Debug)]
pub struct DeployStrParams<'a> {
    /// Path to secret key file.
    pub secret_key: &'a str,
    /// RFC3339-like formatted timestamp. e.g. `2018-02-16T00:31:37Z`.
    ///
    /// If `timestamp` is empty, the current time will be used. Note that timestamp is UTC, not
    /// local.
    ///
    /// See
    /// [the `humantime` docs](https://docs.rs/humantime/latest/humantime/fn.parse_rfc3339_weak.html)
    /// for more information.
    pub timestamp: &'a str,
    /// Time that the `Deploy` will remain valid for.
    ///
    /// A `Deploy` can only be included in a `Block` between `timestamp` and `timestamp + ttl`.
    /// Input examples: '1hr 12min', '30min 50sec', '1day'.
    ///
    /// See
    /// [the `humantime` docs](https://docs.rs/humantime/latest/humantime/fn.parse_duration.html)
    /// for more information.
    pub ttl: &'a str,
    /// Conversion rate between the cost of Wasm opcodes and the motes sent by the payment code.
    pub gas_price: &'a str,
    /// Hex-encoded `Deploy` hashes of deploys which must be executed before this one.
    pub dependencies: Vec<&'a str>,
    /// Name of the chain, to avoid the `Deploy` from being accidentally or maliciously included in
    /// a different chain.
    pub chain_name: &'a str,
}

impl<'a> TryInto<DeployParams> for DeployStrParams<'a> {
    type Error = Error;

    fn try_into(self) -> Result<DeployParams> {
        let DeployStrParams {
            secret_key,
            timestamp,
            ttl,
            gas_price,
            dependencies,
            chain_name,
        } = self;
        parsing::parse_deploy_params(
            secret_key,
            timestamp,
            ttl,
            gas_price,
            &dependencies,
            chain_name,
        )
    }
}

/// Container for payment-related arguments used while constructing a `Deploy`.
///
/// ## `payment_args_simple`
///
/// For methods taking `payment_args_simple`, this parameter is the payment contract arguments, in
/// the form `<NAME:TYPE='VALUE'>` or `<NAME:TYPE=null>`.
///
/// It can only be used with the following simple `CLType`s: bool, i32, i64, u8, u32, u64, u128,
/// u256, u512, unit, string, key, account_hash, uref, public_key and `Option` of each of these.
///
/// Example inputs are:
///
/// ```text
/// name_01:bool='false'
/// name_02:i32='-1'
/// name_03:i64='-2'
/// name_04:u8='3'
/// name_05:u32='4'
/// name_06:u64='5'
/// name_07:u128='6'
/// name_08:u256='7'
/// name_09:u512='8'
/// name_10:unit=''
/// name_11:string='a value'
/// key_account_name:key='account-hash-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20'
/// key_hash_name:key='hash-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20'
/// key_uref_name:key='uref-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20-000'
/// account_hash_name:account_hash='account-hash-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20'
/// uref_name:uref='uref-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20-007'
/// public_key_name:public_key='0119bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1'
/// ```
///
/// For optional values of any these types, prefix the type with "opt_" and use the term "null"
/// without quotes to specify a None value:
///
/// ```text
/// name_01:opt_bool='true'       # Some(true)
/// name_02:opt_bool='false'      # Some(false)
/// name_03:opt_bool=null         # None
/// name_04:opt_i32='-1'          # Some(-1)
/// name_05:opt_i32=null          # None
/// name_06:opt_unit=''           # Some(())
/// name_07:opt_unit=null         # None
/// name_08:opt_string='a value'  # Some("a value".to_string())
/// name_09:opt_string='null'     # Some("null".to_string())
/// name_10:opt_string=null       # None
/// ```
///
/// To get a list of supported types, call
/// [`supported_cl_type_list()`](help/fn.supported_cl_type_list.html). To get this list of examples
/// for supported types, call
/// [`supported_cl_type_examples()`](help/fn.supported_cl_type_examples.html).
///
/// ## `payment_args_complex`
///
/// For methods taking `payment_args_complex`, this parameter is the payment contract arguments, in
/// the form of a `ToBytes`-encoded file.
///
/// ---
///
/// **Note** while multiple payment args can be specified for a single payment code instance, only
/// one of `payment_args_simple` and `payment_args_complex` may be used.
#[derive(Default)]
pub struct PaymentStrParams<'a> {
    payment_amount: &'a str,
    payment_hash: &'a str,
    payment_name: &'a str,
    payment_package_hash: &'a str,
    payment_package_name: &'a str,
    payment_path: &'a str,
    payment_args_simple: Vec<&'a str>,
    payment_args_complex: &'a str,
    payment_version: &'a str,
    payment_entry_point: &'a str,
}

impl<'a> TryInto<ExecutableDeployItem> for PaymentStrParams<'a> {
    type Error = Error;

    fn try_into(self) -> Result<ExecutableDeployItem> {
        let PaymentStrParams {
            payment_amount,
            payment_hash,
            payment_name,
            payment_package_hash,
            payment_package_name,
            payment_path,
            payment_args_simple,
            payment_args_complex,
            payment_version,
            payment_entry_point,
        } = self;

        parsing::parse_payment_info(
            payment_amount,
            payment_hash,
            payment_name,
            payment_package_hash,
            payment_package_name,
            payment_path,
            &payment_args_simple,
            payment_args_complex,
            payment_version,
            payment_entry_point,
        )
    }
}

impl<'a> PaymentStrParams<'a> {
    /// Constructs a `PaymentStrParams` using a payment smart contract file.
    ///
    /// * `payment_path` is the path to the compiled Wasm payment code.
    /// * See the struct docs for a description of [`payment_args_simple`](#payment_args_simple) and
    ///   [`payment_args_complex`](#payment_args_complex).
    pub fn with_path(
        payment_path: &'a str,
        payment_args_simple: Vec<&'a str>,
        payment_args_complex: &'a str,
    ) -> Self {
        Self {
            payment_path,
            payment_args_simple,
            payment_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `PaymentStrParams` using a payment amount.
    ///
    /// `payment_amount` uses the standard-payment system contract rather than custom payment Wasm.
    /// The value is the 'amount' arg of the standard-payment contract.
    pub fn with_amount(payment_amount: &'a str) -> Self {
        Self {
            payment_amount,
            ..Default::default()
        }
    }

    /// Constructs a `PaymentStrParams` using a stored contract's name.
    ///
    /// * `payment_name` is the name of the stored contract (associated with the executing account)
    ///   to be called as the payment.
    /// * `payment_entry_point` is the name of the method that will be used when calling the payment
    ///   contract.
    /// * See the struct docs for a description of [`payment_args_simple`](#payment_args_simple) and
    ///   [`payment_args_complex`](#payment_args_complex).
    pub fn with_name(
        payment_name: &'a str,
        payment_entry_point: &'a str,
        payment_args_simple: Vec<&'a str>,
        payment_args_complex: &'a str,
    ) -> Self {
        Self {
            payment_name,
            payment_entry_point,
            payment_args_simple,
            payment_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `PaymentStrParams` using a stored contract's hex-encoded hash.
    ///
    /// * `payment_hash` is the hex-encoded hash of the stored contract to be called as the payment.
    /// * `payment_entry_point` is the name of the method that will be used when calling the payment
    ///   contract.
    /// * See the struct docs for a description of [`payment_args_simple`](#payment_args_simple) and
    ///   [`payment_args_complex`](#payment_args_complex).
    pub fn with_hash(
        payment_hash: &'a str,
        payment_entry_point: &'a str,
        payment_args_simple: Vec<&'a str>,
        payment_args_complex: &'a str,
    ) -> Self {
        Self {
            payment_hash,
            payment_entry_point,
            payment_args_simple,
            payment_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `PaymentStrParams` using a stored contract's package name.
    ///
    /// * `payment_package_name` is the name of the stored package to be called as the payment.
    /// * `payment_version` is the version of the called payment contract. The latest will be used
    ///   if `payment_version` is empty.
    /// * `payment_entry_point` is the name of the method that will be used when calling the payment
    ///   contract.
    /// * See the struct docs for a description of [`payment_args_simple`](#payment_args_simple) and
    ///   [`payment_args_complex`](#payment_args_complex).
    pub fn with_package_name(
        payment_package_name: &'a str,
        payment_version: &'a str,
        payment_entry_point: &'a str,
        payment_args_simple: Vec<&'a str>,
        payment_args_complex: &'a str,
    ) -> Self {
        Self {
            payment_package_name,
            payment_version,
            payment_entry_point,
            payment_args_simple,
            payment_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `PaymentStrParams` using a stored contract's package hash.
    ///
    /// * `payment_package_hash` is the hex-encoded hash of the stored package to be called as the
    ///   payment.
    /// * `payment_version` is the version of the called payment contract. The latest will be used
    ///   if `payment_version` is empty.
    /// * `payment_entry_point` is the name of the method that will be used when calling the payment
    ///   contract.
    /// * See the struct docs for a description of [`payment_args_simple`](#payment_args_simple) and
    ///   [`payment_args_complex`](#payment_args_complex).
    pub fn with_package_hash(
        payment_package_hash: &'a str,
        payment_version: &'a str,
        payment_entry_point: &'a str,
        payment_args_simple: Vec<&'a str>,
        payment_args_complex: &'a str,
    ) -> Self {
        Self {
            payment_package_hash,
            payment_version,
            payment_entry_point,
            payment_args_simple,
            payment_args_complex,
            ..Default::default()
        }
    }
}

impl<'a> TryInto<ExecutableDeployItem> for SessionStrParams<'a> {
    type Error = Error;

    fn try_into(self) -> Result<ExecutableDeployItem> {
        let SessionStrParams {
            session_hash,
            session_name,
            session_package_hash,
            session_package_name,
            session_path,
            session_args_simple,
            session_args_complex,
            session_version,
            session_entry_point,
            is_session_transfer,
        } = self;

        parsing::parse_session_info(
            session_hash,
            session_name,
            session_package_hash,
            session_package_name,
            session_path,
            &session_args_simple,
            session_args_complex,
            session_version,
            session_entry_point,
            is_session_transfer,
        )
    }
}

/// Container for session-related arguments used while constructing a `Deploy`.
///
/// ## `session_args_simple`
///
/// For methods taking `session_args_simple`, this parameter is the session contract arguments, in
/// the form `<NAME:TYPE='VALUE'>` or `<NAME:TYPE=null>`.
///
/// There are further details in
/// [the docs for the equivalent
/// `payment_args_simple`](struct.PaymentStrParams.html#payment_args_simple).
///
/// ## `session_args_complex`
///
/// For methods taking `session_args_complex`, this parameter is the session contract arguments, in
/// the form of a `ToBytes`-encoded file.
///
/// ---
///
/// **Note** while multiple payment args can be specified for a single session code instance, only
/// one of `session_args_simple` and `session_args_complex` may be used.
#[derive(Default)]
pub struct SessionStrParams<'a> {
    session_hash: &'a str,
    session_name: &'a str,
    session_package_hash: &'a str,
    session_package_name: &'a str,
    session_path: &'a str,
    session_args_simple: Vec<&'a str>,
    session_args_complex: &'a str,
    session_version: &'a str,
    session_entry_point: &'a str,
    is_session_transfer: bool,
}

impl<'a> SessionStrParams<'a> {
    /// Constructs a `SessionStrParams` using a session smart contract file.
    ///
    /// * `session_path` is the path to the compiled Wasm session code.
    /// * See the struct docs for a description of [`session_args_simple`](#session_args_simple) and
    ///   [`session_args_complex`](#session_args_complex).
    pub fn with_path(
        session_path: &'a str,
        session_args_simple: Vec<&'a str>,
        session_args_complex: &'a str,
    ) -> Self {
        Self {
            session_path,
            session_args_simple,
            session_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `SessionStrParams` using a stored contract's name.
    ///
    /// * `session_name` is the name of the stored contract (associated with the executing account)
    ///   to be called as the session.
    /// * `session_entry_point` is the name of the method that will be used when calling the session
    ///   contract.
    /// * See the struct docs for a description of [`session_args_simple`](#session_args_simple) and
    ///   [`session_args_complex`](#session_args_complex).
    pub fn with_name(
        session_name: &'a str,
        session_entry_point: &'a str,
        session_args_simple: Vec<&'a str>,
        session_args_complex: &'a str,
    ) -> Self {
        Self {
            session_name,
            session_entry_point,
            session_args_simple,
            session_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `SessionStrParams` using a stored contract's hex-encoded hash.
    ///
    /// * `session_hash` is the hex-encoded hash of the stored contract to be called as the session.
    /// * `session_entry_point` is the name of the method that will be used when calling the session
    ///   contract.
    /// * See the struct docs for a description of [`session_args_simple`](#session_args_simple) and
    ///   [`session_args_complex`](#session_args_complex).
    pub fn with_hash(
        session_hash: &'a str,
        session_entry_point: &'a str,
        session_args_simple: Vec<&'a str>,
        session_args_complex: &'a str,
    ) -> Self {
        Self {
            session_hash,
            session_entry_point,
            session_args_simple,
            session_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `SessionStrParams` using a stored contract's package name.
    ///
    /// * `session_package_name` is the name of the stored package to be called as the session.
    /// * `session_version` is the version of the called session contract. The latest will be used
    ///   if `session_version` is empty.
    /// * `session_entry_point` is the name of the method that will be used when calling the session
    ///   contract.
    /// * See the struct docs for a description of [`session_args_simple`](#session_args_simple) and
    ///   [`session_args_complex`](#session_args_complex).
    pub fn with_package_name(
        session_package_name: &'a str,
        session_version: &'a str,
        session_entry_point: &'a str,
        session_args_simple: Vec<&'a str>,
        session_args_complex: &'a str,
    ) -> Self {
        Self {
            session_package_name,
            session_version,
            session_entry_point,
            session_args_simple,
            session_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `SessionStrParams` using a stored contract's package hash.
    ///
    /// * `session_package_hash` is the hex-encoded hash of the stored package to be called as the
    ///   session.
    /// * `session_version` is the version of the called session contract. The latest will be used
    ///   if `session_version` is empty.
    /// * `session_entry_point` is the name of the method that will be used when calling the session
    ///   contract.
    /// * See the struct docs for a description of [`session_args_simple`](#session_args_simple) and
    ///   [`session_args_complex`](#session_args_complex).
    pub fn with_package_hash(
        session_package_hash: &'a str,
        session_version: &'a str,
        session_entry_point: &'a str,
        session_args_simple: Vec<&'a str>,
        session_args_complex: &'a str,
    ) -> Self {
        Self {
            session_package_hash,
            session_version,
            session_entry_point,
            session_args_simple,
            session_args_complex,
            ..Default::default()
        }
    }

    /// Constructs a `SessionStrParams` representing a `Transfer` type of `Deploy`.
    ///
    /// * See the struct docs for a description of [`session_args_simple`](#session_args_simple) and
    ///   [`session_args_complex`](#session_args_complex).
    pub fn with_transfer(session_args_simple: Vec<&'a str>, session_args_complex: &'a str) -> Self {
        Self {
            is_session_transfer: true,
            session_args_simple,
            session_args_complex,
            ..Default::default()
        }
    }
}

/// When `verbosity_level` is `1`, the value will be printed to `stdout` with long string fields
/// (e.g. hex-formatted raw Wasm bytes) shortened to a string indicating the char count of the
/// field.  When `verbosity_level` is greater than `1`, the value will be printed to `stdout` with
/// no abbreviation of long fields.  When `verbosity_level` is `0`, the value will not be printed to
/// `stdout`.
pub fn pretty_print_at_level<T: ?Sized + Serialize>(value: &T, verbosity_level: u64) {
    match verbosity_level {
        0 => (),
        1 => {
            println!(
                "{}",
                casper_types::json_pretty_print(value).expect("should encode to JSON")
            );
        }
        _ => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).expect("should encode to JSON")
            );
        }
    }
}

#[cfg(test)]
mod param_tests {
    use super::*;

    #[derive(Debug)]
    struct ErrWrapper(pub Error);

    impl PartialEq for ErrWrapper {
        fn eq(&self, other: &ErrWrapper) -> bool {
            format!("{:?}", self.0) == format!("{:?}", other.0)
        }
    }

    impl From<Error> for ErrWrapper {
        fn from(error: Error) -> Self {
            ErrWrapper(error)
        }
    }

    const HASH: &str = "09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6";
    const NAME: &str = "name";
    const PKG_NAME: &str = "pkg_name";
    const PKG_HASH: &str = "09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6";
    const ENTRYPOINT: &str = "entrypoint";
    const VERSION: &str = "0.1.0";

    fn args_simple() -> Vec<&'static str> {
        vec!["name_01:bool='false'", "name_02:u32='42'"]
    }

    /// Sample data creation methods for PaymentStrParams
    mod session_params {
        use std::collections::BTreeMap;

        use casper_types::CLValue;

        use super::*;

        #[test]
        pub fn with_hash() {
            let params: Result<ExecutableDeployItem> =
                SessionStrParams::with_hash(HASH, ENTRYPOINT, args_simple(), "").try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredContractByHash { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_name() {
            let params: Result<ExecutableDeployItem> =
                SessionStrParams::with_name(NAME, ENTRYPOINT, args_simple(), "").try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredContractByName { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_package_name() {
            let params: Result<ExecutableDeployItem> = SessionStrParams::with_package_name(
                PKG_NAME,
                VERSION,
                ENTRYPOINT,
                args_simple(),
                "",
            )
            .try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredVersionedContractByName { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_package_hash() {
            let params: Result<ExecutableDeployItem> = SessionStrParams::with_package_hash(
                PKG_HASH,
                VERSION,
                ENTRYPOINT,
                args_simple(),
                "",
            )
            .try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredVersionedContractByHash { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }
    }

    /// Sample data creation methods for PaymentStrParams
    mod payment_params {
        use std::collections::BTreeMap;

        use casper_types::CLValue;

        use super::*;

        #[test]
        pub fn with_amount() {
            let params: Result<ExecutableDeployItem> =
                PaymentStrParams::with_amount("100").try_into();
            match params {
                Ok(item @ ExecutableDeployItem::ModuleBytes { .. }) => {
                    let amount = CLValue::from_t(U512::from(100)).unwrap();
                    assert_eq!(item.args().get("amount"), Some(&amount));
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_hash() {
            let params: Result<ExecutableDeployItem> =
                PaymentStrParams::with_hash(HASH, ENTRYPOINT, args_simple(), "").try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredContractByHash { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_name() {
            let params: Result<ExecutableDeployItem> =
                PaymentStrParams::with_name(NAME, ENTRYPOINT, args_simple(), "").try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredContractByName { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_package_name() {
            let params: Result<ExecutableDeployItem> = PaymentStrParams::with_package_name(
                PKG_NAME,
                VERSION,
                ENTRYPOINT,
                args_simple(),
                "",
            )
            .try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredVersionedContractByName { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }

        #[test]
        pub fn with_package_hash() {
            let params: Result<ExecutableDeployItem> = PaymentStrParams::with_package_hash(
                PKG_HASH,
                VERSION,
                ENTRYPOINT,
                args_simple(),
                "",
            )
            .try_into();
            match params {
                Ok(item @ ExecutableDeployItem::StoredVersionedContractByHash { .. }) => {
                    let actual: BTreeMap<String, CLValue> = item.args().clone().into();
                    let mut expected = BTreeMap::new();
                    expected.insert("name_01".to_owned(), CLValue::from_t(false).unwrap());
                    expected.insert("name_02".to_owned(), CLValue::from_t(42u32).unwrap());
                    assert_eq!(actual, expected);
                }
                other => panic!("incorrect type parsed {:?}", other),
            }
        }
    }

    mod deploy_str_params {
        use humantime::{DurationError, TimestampError};

        use super::*;

        use std::{convert::TryInto, result::Result as StdResult};

        use crate::DeployStrParams;

        fn test_value() -> DeployStrParams<'static> {
            DeployStrParams {
                secret_key: "../resources/local/secret_keys/node-1.pem",
                ttl: "10s",
                chain_name: "casper-test-chain-name-1",
                gas_price: "1",
                ..Default::default()
            }
        }

        #[test]
        fn should_convert_into_deploy_params() {
            let deploy_params: StdResult<DeployParams, ErrWrapper> =
                test_value().try_into().map_err(ErrWrapper);
            assert!(deploy_params.is_ok());
        }

        #[test]
        fn should_fail_to_convert_with_bad_timestamp() {
            let mut params = test_value();
            params.timestamp = "garbage";
            let result: StdResult<DeployParams, Error> = params.try_into();
            let result = result.map(|_| ()).map_err(ErrWrapper);
            assert_eq!(
                result,
                Err(
                    Error::FailedToParseTimestamp("timestamp", TimestampError::InvalidFormat)
                        .into()
                )
            );
        }

        #[test]
        fn should_fail_to_convert_with_bad_gas_price() {
            let mut params = test_value();
            params.gas_price = "fifteen";
            let result: StdResult<DeployParams, Error> = params.try_into();
            let result = result.map(|_| ());
            if let Err(Error::FailedToParseInt(context, _)) = result {
                assert_eq!(context, "gas_price");
            } else {
                panic!("should be an error");
            }
        }

        #[test]
        fn should_fail_to_convert_with_bad_chain_name() {
            let mut params = test_value();
            params.chain_name = "";
            let result: StdResult<DeployParams, Error> = params.try_into();
            let result = result.map(|_| ()).map_err(ErrWrapper);
            assert_eq!(result, Ok(()));
        }

        #[test]
        fn should_fail_to_convert_with_bad_ttl() {
            let mut params = test_value();
            params.ttl = "not_a_ttl";
            let result: StdResult<DeployParams, Error> = params.try_into();
            let result = result.map(|_| ()).map_err(ErrWrapper);
            assert_eq!(
                result,
                Err(Error::FailedToParseTimeDiff("ttl", DurationError::NumberExpected(0)).into())
            );
        }

        #[test]
        fn should_fail_to_convert_with_bad_secret_key_path() {
            let mut params = test_value();
            params.secret_key = "";
            let result: StdResult<DeployParams, Error> = params.try_into();
            let result = result.map(|_| ());
            if let Err(Error::CryptoError { context, .. }) = result {
                assert_eq!(context, "secret_key");
            } else {
                panic!("should be an error")
            }
        }

        #[test]
        fn should_fail_to_convert_with_bad_dependencies() {
            use casper_node::crypto::Error as CryptoError;
            let mut params = test_value();
            params.dependencies = vec!["invalid dep"];
            let result: StdResult<DeployParams, Error> = params.try_into();
            let result = result.map(|_| ()).map_err(ErrWrapper);
            assert_eq!(
                result,
                Err(Error::CryptoError {
                    context: "dependencies",
                    error: CryptoError::FromHex(hex::FromHexError::OddLength)
                }
                .into())
            );
        }
    }
}
