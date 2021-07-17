//! A library to support testing of Wasm smart contracts for use on the Casper Platform.
//!
//! # Example
//! Consider a contract held in "contract.wasm" which stores an arbitrary `String` under a `Key`
//! named "special_value":
//! ```no_run
//! use casper_contract::contract_api::{runtime, storage};
//! use casper_types::Key;
//! const KEY: &str = "special_value";
//! const ARG_VALUE: &str = "value";
//!
//! #[no_mangle]
//! pub extern "C" fn call() {
//!     let value: String = runtime::get_named_arg(ARG_VALUE);
//!     let value_ref = storage::new_uref(value);
//!     let value_key: Key = value_ref.into();
//!     runtime::put_key(KEY, value_key);
//! }
//! ```
//!
//! The test could be written as follows:
//! ```no_run
//! use casper_engine_test_support::{Code, Error, SessionBuilder, TestContextBuilder, Value};
//! use casper_types::{U512, RuntimeArgs, runtime_args, PublicKey, account::AccountHash, SecretKey};
//!
//!
//! const MY_ACCOUNT: [u8; 32] = [7u8; 32];
//! const MY_ADDR: [u8; 32] = [8u8; 32];
//! const KEY: &str = "special_value";
//! const VALUE: &str = "hello world";
//! const ARG_MESSAGE: &str = "message";
//!
//! let secret_key = SecretKey::ed25519_from_bytes(MY_ACCOUNT).unwrap();
//! let public_key = PublicKey::from(&secret_key);
//! let account_addr = AccountHash::new(MY_ADDR);
//!
//! let mut context = TestContextBuilder::new()
//!     .with_public_key(public_key, U512::from(128_000_000_000_000u64))
//!     .build();
//!
//! // The test framework checks for compiled Wasm files in '<current working dir>/wasm'.  Paths
//! // relative to the current working dir (e.g. 'wasm/contract.wasm') can also be used, as can
//! // absolute paths.
//! let session_code = Code::from("contract.wasm");
//! let session_args = runtime_args! {
//!     ARG_MESSAGE => VALUE,
//! };
//! let session = SessionBuilder::new(session_code, session_args)
//!     .with_address(account_addr)
//!     .with_authorization_keys(&[account_addr])
//!     .build();
//!
//! let result_of_query: Result<Value, Error> = context.run(session).query(account_addr, &[KEY.to_string()]);
//!
//! let returned_value = result_of_query.expect("should be a value");
//!
//! let expected_value = Value::from_t(VALUE.to_string()).expect("should construct Value");
//! assert_eq!(expected_value, returned_value);
//! ```

#![doc(html_root_url = "https://docs.rs/casper-engine-test-support/1.3.0")]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Favicon_RGB_50px.png",
    html_logo_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Symbol_RGB.png",
    test(attr(forbid(warnings)))
)]
#![warn(missing_docs)]

mod account;
mod code;
mod error;
// This module is not intended to be used by third party crates.
#[doc(hidden)]
pub mod internal;
mod session;
mod test_context;
mod value;

pub use account::Account;
pub use casper_types::account::AccountHash;
pub use code::Code;
pub use error::{Error, Result};
pub use session::{Session, SessionBuilder, SessionTransferInfo};
pub use test_context::{TestContext, TestContextBuilder};
pub use value::Value;

/// The address of a [`URef`](casper_types::URef) (unforgeable reference) on the network.
pub type URefAddr = [u8; 32];

/// The hash of a smart contract stored on the network, which can be used to reference the contract.
pub type Hash = [u8; 32];

/// Default test account address.
pub use crate::internal::DEFAULT_ACCOUNT_ADDR;

/// Default initial balance of a test account in motes.
pub const DEFAULT_ACCOUNT_INITIAL_BALANCE: u64 = 100_000_000_000_000_000u64;

/// Minimal amount for a transfer that creates new accounts.
pub const MINIMUM_ACCOUNT_CREATION_BALANCE: u64 = 7_500_000_000_000_000u64;
