//! Contract Runtime component.
mod config;
mod error;
mod operations;
mod types;

use std::{
    collections::VecDeque,
    fmt::{self, Debug, Formatter},
    sync::Arc,
    time::Instant,
};

pub use config::Config;
pub use error::{BlockExecutionError, ConfigError};
pub use types::{BlockAndExecutionEffects, EraValidatorsRequest, ValidatorWeightsByEraIdRequest};

use datasize::DataSize;
use lmdb::DatabaseFlags;
use prometheus::{self, Histogram, HistogramOpts, IntGauge, Registry};
use serde::Serialize;
use tracing::{debug, error, info, trace};

use casper_execution_engine::{
    core::engine_state::{
        self, genesis::GenesisSuccess, EngineConfig, EngineState, GetEraValidatorsError,
        GetEraValidatorsRequest,
    },
    shared::{
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
    },
    storage::{
        global_state::lmdb::LmdbGlobalState, protocol_data_store::lmdb::LmdbProtocolDataStore,
        transaction_source::lmdb::LmdbEnvironment, trie::Trie, trie_store::lmdb::LmdbTrieStore,
    },
};
use casper_types::{Key, ProtocolVersion};

use crate::{
    components::Component,
    crypto::hash::Digest,
    effect::{
        announcements::{ContractRuntimeAnnouncement, ControlAnnouncement},
        requests::ContractRuntimeRequest,
        EffectBuilder, EffectExt, Effects,
    },
    fatal,
    types::{BlockHash, BlockHeader, Chainspec, Deploy, FinalizedBlock},
    utils::WithDir,
    NodeRng, StorageConfig,
};

/// State to use to construct the next block in the blockchain. Includes the state root hash for the
/// execution engine as well as certain values the next header will be based on.
#[derive(DataSize, Debug, Clone, Serialize)]
pub struct ExecutionPreState {
    /// The height of the next `Block` to be constructed. Note that this must match the height of
    /// the `FinalizedBlock` used to generate the block.
    next_block_height: u64,
    /// The state root to use when executing deploys.
    pre_state_root_hash: Digest,
    /// The parent hash of the next `Block`.
    parent_hash: BlockHash,
    /// The accumulated seed for the pseudo-random number generator to be incorporated into the
    /// next `Block`, where additional entropy will be introduced.
    parent_seed: Digest,
}

impl ExecutionPreState {
    pub(crate) fn new(
        pre_state_root_hash: Digest,
        next_block_height: u64,
        parent_hash: BlockHash,
        parent_seed: Digest,
    ) -> Self {
        ExecutionPreState {
            pre_state_root_hash,
            next_block_height,
            parent_hash,
            parent_seed,
        }
    }

    /// Get the next block height according that will succeed the block specified by `parent_hash`.
    pub fn next_block_height(&self) -> u64 {
        self.next_block_height
    }
}

impl From<&BlockHeader> for ExecutionPreState {
    fn from(block_header: &BlockHeader) -> Self {
        ExecutionPreState {
            pre_state_root_hash: *block_header.state_root_hash(),
            next_block_height: block_header.height() + 1,
            parent_hash: block_header.hash(),
            parent_seed: block_header.accumulated_seed(),
        }
    }
}

/// The contract runtime components.
#[derive(DataSize)]
pub struct ContractRuntime {
    execution_pre_state: ExecutionPreState,
    engine_state: Arc<EngineState<LmdbGlobalState>>,
    metrics: Arc<ContractRuntimeMetrics>,
    protocol_version: ProtocolVersion,

    /// Finalized blocks waiting for their pre-state hash to start executing.
    exec_queue: VecDeque<(FinalizedBlock, Vec<Deploy>)>,
}

impl Debug for ContractRuntime {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContractRuntime").finish()
    }
}

/// Metrics for the contract runtime component.
#[derive(Debug)]
pub struct ContractRuntimeMetrics {
    run_execute: Histogram,
    apply_effect: Histogram,
    commit_upgrade: Histogram,
    run_query: Histogram,
    commit_step: Histogram,
    get_balance: Histogram,
    get_validator_weights: Histogram,
    get_era_validators: Histogram,
    get_bids: Histogram,
    missing_trie_keys: Histogram,
    put_trie: Histogram,
    read_trie: Histogram,
    chain_height: IntGauge,
}

/// Value of upper bound of histogram.
const EXPONENTIAL_BUCKET_START: f64 = 0.01;

/// Multiplier of previous upper bound for next bound.
const EXPONENTIAL_BUCKET_FACTOR: f64 = 2.0;

/// Bucket count, with the last bucket going to +Inf which will not be included in the results.
/// - start = 0.01, factor = 2.0, count = 10
/// - start * factor ^ count = 0.01 * 2.0 ^ 10 = 10.24
/// - Values above 10.24 (f64 seconds here) will not fall in a bucket that is kept.
const EXPONENTIAL_BUCKET_COUNT: usize = 10;

const RUN_EXECUTE_NAME: &str = "contract_runtime_run_execute";
const RUN_EXECUTE_HELP: &str = "tracking run of engine_state.run_execute in seconds.";
const APPLY_EFFECT_NAME: &str = "contract_runtime_apply_commit";
const APPLY_EFFECT_HELP: &str = "tracking run of engine_state.apply_effect in seconds.";
const RUN_QUERY_NAME: &str = "contract_runtime_run_query";
const RUN_QUERY_HELP: &str = "tracking run of engine_state.run_query in seconds.";
const COMMIT_STEP_NAME: &str = "contract_runtime_commit_step";
const COMMIT_STEP_HELP: &str = "tracking run of engine_state.commit_step in seconds.";
const COMMIT_UPGRADE_NAME: &str = "contract_runtime_commit_upgrade";
const COMMIT_UPGRADE_HELP: &str = "tracking run of engine_state.commit_upgrade in seconds";
const GET_BALANCE_NAME: &str = "contract_runtime_get_balance";
const GET_BALANCE_HELP: &str = "tracking run of engine_state.get_balance in seconds.";
const GET_VALIDATOR_WEIGHTS_NAME: &str = "contract_runtime_get_validator_weights";
const GET_VALIDATOR_WEIGHTS_HELP: &str =
    "tracking run of engine_state.get_validator_weights in seconds.";
const GET_ERA_VALIDATORS_NAME: &str = "contract_runtime_get_era_validators";
const GET_ERA_VALIDATORS_HELP: &str = "tracking run of engine_state.get_era_validators in seconds.";
const GET_BIDS_NAME: &str = "contract_runtime_get_bids";
const GET_BIDS_HELP: &str = "tracking run of engine_state.get_bids in seconds.";
const READ_TRIE_NAME: &str = "contract_runtime_read_trie";
const READ_TRIE_HELP: &str = "tracking run of engine_state.read_trie in seconds.";
const PUT_TRIE_NAME: &str = "contract_runtime_put_trie";
const PUT_TRIE_HELP: &str = "tracking run of engine_state.put_trie in seconds.";
const MISSING_TRIE_KEYS_NAME: &str = "contract_runtime_missing_trie_keys";
const MISSING_TRIE_KEYS_HELP: &str = "tracking run of engine_state.missing_trie_keys in seconds.";

/// Create prometheus Histogram and register.
fn register_histogram_metric(
    registry: &Registry,
    metric_name: &str,
    metric_help: &str,
) -> Result<Histogram, prometheus::Error> {
    let common_buckets = prometheus::exponential_buckets(
        EXPONENTIAL_BUCKET_START,
        EXPONENTIAL_BUCKET_FACTOR,
        EXPONENTIAL_BUCKET_COUNT,
    )?;
    let histogram_opts = HistogramOpts::new(metric_name, metric_help).buckets(common_buckets);
    let histogram = Histogram::with_opts(histogram_opts)?;
    registry.register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

impl ContractRuntimeMetrics {
    /// Constructor of metrics which creates and registers metrics objects for use.
    fn new(registry: &Registry) -> Result<Self, prometheus::Error> {
        let chain_height = IntGauge::new("chain_height", "current chain height")?;
        registry.register(Box::new(chain_height.clone()))?;
        Ok(ContractRuntimeMetrics {
            chain_height,
            run_execute: register_histogram_metric(registry, RUN_EXECUTE_NAME, RUN_EXECUTE_HELP)?,
            apply_effect: register_histogram_metric(
                registry,
                APPLY_EFFECT_NAME,
                APPLY_EFFECT_HELP,
            )?,
            run_query: register_histogram_metric(registry, RUN_QUERY_NAME, RUN_QUERY_HELP)?,
            commit_step: register_histogram_metric(registry, COMMIT_STEP_NAME, COMMIT_STEP_HELP)?,
            commit_upgrade: register_histogram_metric(
                registry,
                COMMIT_UPGRADE_NAME,
                COMMIT_UPGRADE_HELP,
            )?,
            get_balance: register_histogram_metric(registry, GET_BALANCE_NAME, GET_BALANCE_HELP)?,
            get_validator_weights: register_histogram_metric(
                registry,
                GET_VALIDATOR_WEIGHTS_NAME,
                GET_VALIDATOR_WEIGHTS_HELP,
            )?,
            get_era_validators: register_histogram_metric(
                registry,
                GET_ERA_VALIDATORS_NAME,
                GET_ERA_VALIDATORS_HELP,
            )?,
            get_bids: register_histogram_metric(registry, GET_BIDS_NAME, GET_BIDS_HELP)?,
            read_trie: register_histogram_metric(registry, READ_TRIE_NAME, READ_TRIE_HELP)?,
            put_trie: register_histogram_metric(registry, PUT_TRIE_NAME, PUT_TRIE_HELP)?,
            missing_trie_keys: register_histogram_metric(
                registry,
                MISSING_TRIE_KEYS_NAME,
                MISSING_TRIE_KEYS_HELP,
            )?,
        })
    }
}

impl<REv> Component<REv> for ContractRuntime
where
    REv: From<ContractRuntimeRequest>
        + From<ContractRuntimeAnnouncement>
        + From<ControlAnnouncement>
        + Send,
{
    type Event = ContractRuntimeRequest;
    type ConstructionError = ConfigError;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            ContractRuntimeRequest::GetProtocolData {
                protocol_version,
                responder,
            } => {
                let result = self
                    .engine_state
                    .get_protocol_data(protocol_version)
                    .map(|inner| inner.map(Box::new));

                responder.respond(result).ignore()
            }
            ContractRuntimeRequest::CommitGenesis {
                chainspec,
                responder,
            } => {
                let result = self.commit_genesis(chainspec);
                responder.respond(result).ignore()
            }
            ContractRuntimeRequest::Upgrade {
                upgrade_config,
                responder,
            } => {
                debug!(?upgrade_config, "upgrade");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let result = engine_state.commit_upgrade(correlation_id, *upgrade_config);
                    metrics
                        .commit_upgrade
                        .observe(start.elapsed().as_secs_f64());
                    debug!(?result, "upgrade result");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::Query {
                query_request,
                responder,
            } => {
                trace!(?query_request, "query");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let result = engine_state.run_query(correlation_id, query_request);
                    metrics.run_query.observe(start.elapsed().as_secs_f64());
                    trace!(?result, "query result");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::GetBalance {
                balance_request,
                responder,
            } => {
                trace!(?balance_request, "balance");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let result = engine_state.get_purse_balance(
                        correlation_id,
                        balance_request.state_hash(),
                        balance_request.purse_uref(),
                    );
                    metrics.get_balance.observe(start.elapsed().as_secs_f64());
                    trace!(?result, "balance result");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::IsBonded {
                state_root_hash,
                era_id,
                protocol_version,
                public_key: validator_key,
                responder,
            } => {
                trace!(era=%era_id, public_key = %validator_key, "is validator bonded request");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                let request =
                    GetEraValidatorsRequest::new(state_root_hash.into(), protocol_version);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let era_validators = engine_state.get_era_validators(correlation_id, request);
                    metrics
                        .get_validator_weights
                        .observe(start.elapsed().as_secs_f64());
                    trace!(?era_validators, "is validator bonded result");
                    let is_bonded =
                        era_validators.and_then(|validator_map| match validator_map.get(&era_id) {
                            None => Err(GetEraValidatorsError::EraValidatorsMissing),
                            Some(era_validators) => Ok(era_validators.contains_key(&validator_key)),
                        });
                    responder.respond(is_bonded).await
                }
                .ignore()
            }
            ContractRuntimeRequest::GetEraValidators { request, responder } => {
                trace!(?request, "get era validators request");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                // Increment the counter to track the amount of times GetEraValidators was
                // requested.
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let era_validators =
                        engine_state.get_era_validators(correlation_id, request.into());
                    metrics
                        .get_era_validators
                        .observe(start.elapsed().as_secs_f64());
                    trace!(?era_validators, "get era validators response");
                    responder.respond(era_validators).await
                }
                .ignore()
            }
            ContractRuntimeRequest::ReadTrie {
                trie_key,
                responder,
            } => {
                trace!(?trie_key, "read_trie request");
                let result = self.read_trie(trie_key);
                async move {
                    let result = match result {
                        Ok(result) => result,
                        Err(error) => {
                            error!(?error, "read_trie_request");
                            None
                        }
                    };
                    trace!(?result, "read_trie response");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::PutTrie { trie, responder } => {
                trace!(?trie, "put_trie request");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let result = engine_state
                        .put_trie_and_find_missing_descendant_trie_keys(correlation_id, &*trie);
                    metrics.put_trie.observe(start.elapsed().as_secs_f64());
                    trace!(?result, "put_trie response");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::ExecuteBlock {
                protocol_version,
                execution_pre_state,
                finalized_block,
                deploys,
                responder,
            } => {
                trace!(
                    ?protocol_version,
                    ?execution_pre_state,
                    ?finalized_block,
                    ?deploys,
                    "execute block request"
                );
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let result = operations::execute_finalized_block(
                        engine_state.as_ref(),
                        metrics.as_ref(),
                        protocol_version,
                        execution_pre_state,
                        finalized_block,
                        deploys,
                    );
                    trace!(?result, "execute block response");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::EnqueueBlockForExecution {
                finalized_block,
                deploys,
            } => {
                info!(?finalized_block, "enqueuing finalized block for execution");
                if self.execution_pre_state.next_block_height == finalized_block.height() {
                    self.execute_finalized_block(
                        effect_builder,
                        self.protocol_version,
                        self.execution_pre_state.clone(),
                        finalized_block,
                        deploys,
                    )
                } else {
                    self.exec_queue.push_back((finalized_block, deploys));
                    Effects::new()
                }
            }
            ContractRuntimeRequest::GetBids {
                get_bids_request,
                responder,
            } => {
                trace!(?get_bids_request, "get bids request");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let result = engine_state.get_bids(correlation_id, get_bids_request);
                    metrics.get_bids.observe(start.elapsed().as_secs_f64());
                    trace!(?result, "get bids result");
                    responder.respond(result).await
                }
                .ignore()
            }
            ContractRuntimeRequest::MissingTrieKeys {
                trie_key,
                responder,
            } => {
                trace!(?trie_key, "missing_trie_keys request");
                let engine_state = Arc::clone(&self.engine_state);
                let metrics = Arc::clone(&self.metrics);
                async move {
                    let correlation_id = CorrelationId::new();
                    let start = Instant::now();
                    let result = engine_state.missing_trie_keys(correlation_id, vec![trie_key]);
                    metrics.read_trie.observe(start.elapsed().as_secs_f64());
                    trace!(?result, "missing_trie_keys response");
                    responder.respond(result).await
                }
                .ignore()
            }
        }
    }
}

impl ContractRuntime {
    pub(crate) fn new(
        protocol_version: ProtocolVersion,
        storage_config: WithDir<StorageConfig>,
        contract_runtime_config: &Config,
        registry: &Registry,
    ) -> Result<Self, ConfigError> {
        // TODO: This is bogus, get rid of this
        let execution_pre_state = ExecutionPreState {
            pre_state_root_hash: Default::default(),
            next_block_height: 0,
            parent_hash: Default::default(),
            parent_seed: Default::default(),
        };

        let path = storage_config.with_dir(storage_config.value().path.clone());
        let environment = Arc::new(LmdbEnvironment::new(
            path.as_path(),
            contract_runtime_config.max_global_state_size(),
            contract_runtime_config.max_readers(),
        )?);

        let trie_store = Arc::new(LmdbTrieStore::new(
            &environment,
            None,
            DatabaseFlags::empty(),
        )?);

        let protocol_data_store = Arc::new(LmdbProtocolDataStore::new(
            &environment,
            None,
            DatabaseFlags::empty(),
        )?);

        let global_state = LmdbGlobalState::empty(environment, trie_store, protocol_data_store)?;
        let engine_config = EngineConfig::new(contract_runtime_config.max_query_depth());

        let engine_state = Arc::new(EngineState::new(global_state, engine_config));

        let metrics = Arc::new(ContractRuntimeMetrics::new(registry)?);
        Ok(ContractRuntime {
            execution_pre_state,
            protocol_version,
            exec_queue: VecDeque::new(),
            engine_state,
            metrics,
        })
    }

    /// Commits a genesis using a chainspec
    fn commit_genesis(
        &self,
        chainspec: Arc<Chainspec>,
    ) -> Result<GenesisSuccess, engine_state::Error> {
        let correlation_id = CorrelationId::new();
        let genesis_config_hash = chainspec.hash();
        let protocol_version = chainspec.protocol_config.version;
        // Transforms a chainspec into a valid genesis config for execution engine.
        let ee_config = chainspec.as_ref().into();
        self.engine_state.commit_genesis(
            correlation_id,
            genesis_config_hash.into(),
            protocol_version,
            &ee_config,
        )
    }

    /// Retrieve trie keys for the integrity check.
    pub fn trie_store_check(&self, trie_keys: Vec<Blake2bHash>) -> Vec<Blake2bHash> {
        let correlation_id = CorrelationId::new();
        match self
            .engine_state
            .missing_trie_keys(correlation_id, trie_keys)
        {
            Ok(keys) => keys,
            Err(error) => panic!("Error in retrieving keys for DB check: {:?}", error),
        }
    }

    pub(crate) fn set_initial_state(&mut self, sequential_block_state: ExecutionPreState) {
        self.execution_pre_state = sequential_block_state;
    }

    fn execute_finalized_block<REv>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        protocol_version: ProtocolVersion,
        execution_pre_state: ExecutionPreState,
        finalized_block: FinalizedBlock,
        deploys: Vec<Deploy>,
    ) -> Effects<ContractRuntimeRequest>
    where
        REv: From<ContractRuntimeRequest>
            + From<ContractRuntimeAnnouncement>
            + From<ControlAnnouncement>
            + Send,
    {
        let BlockAndExecutionEffects {
            block,
            execution_results,
            maybe_step_execution_effect,
        } = match operations::execute_finalized_block(
            self.engine_state.as_ref(),
            self.metrics.as_ref(),
            protocol_version,
            execution_pre_state,
            finalized_block,
            deploys,
        ) {
            Ok(block_and_execution_effects) => block_and_execution_effects,
            Err(error) => return fatal!(effect_builder, "{}", error).ignore(),
        };

        self.execution_pre_state = ExecutionPreState::from(block.header());

        let era_id = block.header().era_id();
        let mut effects = effect_builder
            .announce_linear_chain_block(block, execution_results)
            .ignore();
        if let Some(step_execution_effect) = maybe_step_execution_effect {
            effects.extend(
                effect_builder
                    .announce_step_success(era_id, step_execution_effect)
                    .ignore(),
            );
        }

        // If the child is already finalized, start execution.
        if let Some((finalized_block, deploys)) = self.exec_queue.pop_front() {
            effects.extend(
                effect_builder
                    .enqueue_block_for_execution(finalized_block, deploys)
                    .ignore(),
            );
        }
        effects
    }

    /// Read a [Trie<Key, StoredValue>] from the trie store.
    pub fn read_trie(
        &mut self,
        trie_key: Blake2bHash,
    ) -> Result<Option<Trie<Key, StoredValue>>, engine_state::Error> {
        let correlation_id = CorrelationId::new();
        let start = Instant::now();
        let result = self.engine_state.read_trie(correlation_id, trie_key);
        self.metrics
            .read_trie
            .observe(start.elapsed().as_secs_f64());
        result
    }
}
