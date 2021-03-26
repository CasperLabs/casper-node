use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use datasize::DataSize;

use crate::types::TimeDiff;

use super::round_success_meter::config::Config as RSMConfig;

/// Highway-specific configuration.
/// NOTE: This is *NOT* protocol configuration that has to be the same on all nodes.
#[derive(DataSize, Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Path to the folder where unit hash files will be stored.
    pub unit_hashes_folder: PathBuf,
    /// The duration for which incoming vertices with missing dependencies are kept in a queue.
    pub pending_vertex_timeout: TimeDiff,
    /// The frequency at which we will ask peers for their latest state.
    pub request_latest_state_timeout: TimeDiff,
    /// If the current era's protocol state has not progressed for this long, shut down.
    pub standstill_timeout: TimeDiff,
    /// Log inactive or faulty validators periodically, with this interval.
    pub log_participation_interval: TimeDiff,
    /// The maximum number of blocks by which execution is allowed to lag behind finalization.
    /// If it is more than that, consensus will pause, and resume once the executor has caught up.
    pub max_execution_delay: u64,
    pub round_success_meter: RSMConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            unit_hashes_folder: Default::default(),
            pending_vertex_timeout: "10sec".parse().unwrap(),
            request_latest_state_timeout: "5sec".parse().unwrap(),
            standstill_timeout: "1min".parse().unwrap(),
            log_participation_interval: "10sec".parse().unwrap(),
            max_execution_delay: 3,
            round_success_meter: RSMConfig::default(),
        }
    }
}
