//! A Rust library for writing smart contracts on the
//! [Casper Platform](https://techspec.casperlabs.io).
//!
//! # `no_std`
//!
//! By default, the library is `no_std`, however you can enable full `std` functionality by enabling
//! the crate's `std` feature.
//!
//! # Example
//!
//! The following example contains session code which persists an integer value under an unforgeable
//! reference.  It then stores the unforgeable reference under a name in context-local storage.
//!
//! ```rust,no_run
//! #![no_std]
//!
//! use casper_contract::{
//!     contract_api::{runtime, storage},
//! };
//! use casper_types::{Key, URef};
//!
//! const KEY: &str = "special_value";
//! const ARG_VALUE: &str = "value";
//!
//! fn store(value: i32) {
//!     // Store `value` under a new unforgeable reference.
//!     let value_ref: URef = storage::new_uref(value);
//!
//!     // Wrap the unforgeable reference in a value of type `Key`.
//!     let value_key: Key = value_ref.into();
//!
//!     // Store this key under the name "special_value" in context-local storage.
//!     runtime::put_key(KEY, value_key);
//! }
//!
//! // All session code must have a `call` entrypoint.
//! #[no_mangle]
//! pub extern "C" fn call() {
//!     // Get the optional first argument supplied to the argument.
//!     let value: i32 = runtime::get_named_arg(ARG_VALUE);
//!     store(value);
//! }
//! # fn main() {}
//! ```
//!
//! # Writing Smart Contracts
//!
//! Support for writing smart contracts are contained in the [`contract_api`] module and its
//! submodules.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(
    not(feature = "std"),
    feature(alloc_error_handler, core_intrinsics, lang_items)
)]
#![doc(html_root_url = "https://docs.rs/casper-contract/1.3.0")]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Favicon_RGB_50px.png",
    html_logo_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Symbol_RGB.png",
    test(attr(forbid(warnings)))
)]
#![warn(missing_docs)]

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

/// An instance of [`WeeAlloc`](https://docs.rs/wee_alloc) which allows contracts built as `no_std`
/// to avoid having to provide a global allocator themselves.
#[cfg(not(any(feature = "std", test)))]
#[global_allocator]
pub static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

pub mod contract_api;
pub mod ext_ffi;
#[cfg(not(any(feature = "std", test, doc)))]
pub mod handlers;
pub mod unwrap_or_revert;
