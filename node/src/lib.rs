//! # Casper blockchain node
//!
//! This crate contain the core application for the Casper blockchain. Run with `--help` to see
//! available command-line arguments.
//!
//! ## Application structure
//!
//! While the [`main`](fn.main.html) function is the central entrypoint for the node application,
//! its core event loop is found inside the [reactor](reactor/index.html).

#![doc(html_root_url = "https://docs.rs/casper-node/1.3.0")]
#![doc(
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
#![feature(test)]

extern crate test;

pub mod components;
mod config_migration;
pub mod crypto;
mod data_migration;
pub mod effect;
pub mod logging;
pub mod protocol;
pub mod reactor;
#[cfg(test)]
pub mod testing;
pub mod tls;
pub mod types;
pub mod utils;

use std::sync::{
    atomic::{AtomicBool, AtomicUsize},
    Arc,
};

use ansi_term::Color::Red;
use once_cell::sync::Lazy;
#[cfg(not(test))]
use rand::SeedableRng;
use signal_hook::{
    consts::{signal::SIGUSR1, TERM_SIGNALS},
    flag,
};

pub use components::{
    consensus::Config as ConsensusConfig,
    contract_runtime::Config as ContractRuntimeConfig,
    deploy_acceptor::Config as DeployAcceptorConfig,
    event_stream_server::Config as EventStreamServerConfig,
    fetcher::Config as FetcherConfig,
    gossiper::{Config as GossipConfig, Error as GossipError},
    rest_server::Config as RestServerConfig,
    rpc_server::{rpcs, Config as RpcServerConfig},
    small_network::{Config as SmallNetworkConfig, Error as SmallNetworkError},
    storage::{Config as StorageConfig, Error as StorageError},
};
pub use config_migration::{migrate_config, Error as ConfigMigrationError};
pub use data_migration::{migrate_data, Error as DataMigrationError};
pub use types::NodeRng;
pub use utils::OS_PAGE_SIZE;

/// The maximum thread count which should be spawned by the tokio runtime.
pub const MAX_THREAD_COUNT: usize = 512;

fn version_string(color: bool) -> String {
    let mut version = format!("{}-{}", env!("CARGO_PKG_VERSION"), env!("VERGEN_SHA_SHORT"));

    // Add a `@DEBUG` (or similar) tag to release string on non-release builds.
    if env!("NODE_BUILD_PROFILE") != "release" {
        version += "@";
        let profile = env!("NODE_BUILD_PROFILE").to_uppercase();
        version.push_str(&if color {
            Red.paint(&profile).to_string()
        } else {
            profile
        });
    }

    version
}

/// Color version string for the compiled node. Filled in at build time, output allocated at
/// runtime.
pub static VERSION_STRING_COLOR: Lazy<String> = Lazy::new(|| version_string(true));

/// Version string for the compiled node. Filled in at build time, output allocated at runtime.
pub static VERSION_STRING: Lazy<String> = Lazy::new(|| version_string(false));

/// Global value that indicates the currently running reactor should exit if it is non-zero.
pub static TERMINATION_REQUESTED: Lazy<Arc<AtomicUsize>> =
    Lazy::new(|| Arc::new(AtomicUsize::new(0)));

/// Global flag that indicates the currently running reactor should dump its event queue.
pub static QUEUE_DUMP_REQUESTED: Lazy<Arc<AtomicBool>> =
    Lazy::new(|| Arc::new(AtomicBool::new(false)));

/// Setup UNIX signal hooks for current application.
pub fn setup_signal_hooks() {
    for signal in TERM_SIGNALS {
        flag::register_usize(
            *signal,
            Arc::clone(&*TERMINATION_REQUESTED),
            *signal as usize,
        )
        .unwrap_or_else(|error| panic!("failed to register signal {}: {}", signal, error));
    }
    let _ = flag::register(SIGUSR1, Arc::clone(&*QUEUE_DUMP_REQUESTED));
}

/// Constructs a new `NodeRng`.
#[cfg(not(test))]
pub fn new_rng() -> NodeRng {
    NodeRng::from_entropy()
}

/// Constructs a new `NodeRng`.
#[cfg(test)]
pub fn new_rng() -> NodeRng {
    NodeRng::new()
}
