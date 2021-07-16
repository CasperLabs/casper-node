//! Contract Runtime component.
mod config;
mod operations;
mod types;

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt::{self, Debug, Formatter},
    sync::Arc,
    time::Instant,
};

pub use config::Config;
use smallvec::SmallVec;

pub use types::{EraValidatorsRequest, ValidatorWeightsByEraIdRequest};

use datasize::DataSize;
use derive_more::From;
use lmdb::DatabaseFlags;
use prometheus::{self, Histogram, HistogramOpts, IntGauge, Registry};
use thiserror::Error;
use tracing::{debug, error, trace};

use casper_execution_engine::{
    core::engine_state::{
        self, genesis::GenesisResult, step::EvictItem, DeployItem, EngineConfig, EngineState,
        ExecuteRequest, GetEraValidatorsError, GetEraValidatorsRequest, RewardItem, SlashItem,
        StepRequest, StepResult,
    },
    shared::newtypes::{Blake2bHash, CorrelationId},
    storage::{
        error::lmdb::Error as StorageLmdbError, global_state::lmdb::LmdbGlobalState,
        protocol_data_store::lmdb::LmdbProtocolDataStore,
        transaction_source::lmdb::LmdbEnvironment, trie_store::lmdb::LmdbTrieStore,
    },
};
use casper_types::{
    system::auction::ValidatorWeights, ExecutionResult, ProtocolVersion, PublicKey, U512,
};

use crate::{
    components::Component,
    crypto::hash::Digest,
    effect::{
        announcements::ContractRuntimeAnnouncement,
        requests::{ConsensusRequest, ContractRuntimeRequest, LinearChainRequest, StorageRequest},
        EffectBuilder, EffectExt, Effects,
    },
    types::{
        Block, BlockHash, BlockHeader, Chainspec, Deploy, DeployHash, DeployHeader, FinalizedBlock,
        NodeId,
    },
    utils::WithDir,
    NodeRng, StorageConfig,
};

/// Contract runtime component event.
#[derive(Debug, From)]
pub enum Event {
    /// A request made for the contract runtime component.
    #[from]
    Request(Box<ContractRuntimeRequest>),
    /// Indicates that block has already been finalized and executed in the past.
    BlockAlreadyExists(Box<Block>),
    /// Indicates that a block is not known yet, and needs to be executed.
    BlockIsNew(Box<FinalizedBlock>),

    /// Results received by the contract runtime.
    #[from]
    Result(Box<ContractRuntimeResult>),
}

/// Contract runtime component event.
#[derive(Debug, From)]
pub enum ContractRuntimeResult {
    /// Received all requested deploys.
    GetDeploysResult {
        /// The block that needs the deploys for execution.
        finalized_block: FinalizedBlock,
        /// Contents of deploys. All deploys are expected to be present in the storage component.
        deploys: VecDeque<Deploy>,
    },
    /// Received a parent result.
    GetParentResult {
        /// The block that needs the deploys for execution.
        finalized_block: FinalizedBlock,
        /// Contents of deploys. All deploys are expected to be present in the storage component.
        deploys: VecDeque<Deploy>,
        /// Parent of the newly finalized block.
        /// If it's the first block after Genesis then `parent` is `None`.
        parent: Option<(BlockHash, Digest, Digest)>,
    },
    /// The result of running the step on a switch block.
    RunStepResult {
        /// State of this request.
        state: Box<RequestState>,
        /// The result.
        result: Result<StepResult, engine_state::Error>,
    },
    /// Once a block is executed and committed, re-enter evented flow.
    ExecutedAndCommitted(Box<RequestState>),
}

/// Convenience trait for ContractRuntime's accepted event types.
pub trait ReactorEventT:
    From<Event>
    + From<StorageRequest>
    + From<LinearChainRequest<NodeId>>
    + From<ContractRuntimeRequest>
    + From<ContractRuntimeAnnouncement>
    + From<ConsensusRequest>
    + Send
{
}

impl<REv> ReactorEventT for REv where
    REv: From<Event>
        + From<StorageRequest>
        + From<LinearChainRequest<NodeId>>
        + From<ContractRuntimeRequest>
        + From<ContractRuntimeAnnouncement>
        + From<ConsensusRequest>
        + Send
{
}

#[derive(DataSize, Debug)]
struct ExecutedBlockSummary {
    hash: BlockHash,
    state_root_hash: Digest,
    accumulated_seed: Digest,
}

type BlockHeight = u64;

/// The contract runtime components.
#[derive(DataSize)]
pub struct ContractRuntime {
    initial_state: InitialState,
    engine_state: Arc<EngineState<LmdbGlobalState>>,
    metrics: Arc<ContractRuntimeMetrics>,

    protocol_version: ProtocolVersion,

    /// A mapping from block height to executed block's ID and post-state hash, to allow
    /// identification of a parent block's details once a finalized block has been executed.
    ///
    /// The key is a tuple of block's height (it's a linear chain so it's monotonically
    /// increasing), and the `ExecutedBlockSummary` is derived from the executed block.
    parent_map: HashMap<BlockHeight, ExecutedBlockSummary>,

    /// Finalized blocks waiting for their pre-state hash to start executing.
    exec_queue: HashMap<BlockHeight, (FinalizedBlock, VecDeque<Deploy>)>,
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
    get_era_validator_weights_by_era_id: Histogram,
    get_bids: Histogram,
    missing_trie_keys: Histogram,
    put_trie: Histogram,
    read_trie: Histogram,
    /// The current chain height.
    pub chain_height: IntGauge,
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
const GET_ERA_VALIDATORS_WEIGHT_BY_ERA_ID_NAME: &str =
    "contract_runtime_get_era_validator_weights_by_era_id";
const GET_ERA_VALIDATORS_WEIGHT_BY_ERA_ID_HELP: &str =
    "tracking run of engine_state.get_era_validator_weights_by_era_id in seconds.";
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
            get_era_validator_weights_by_era_id: register_histogram_metric(
                registry,
                GET_ERA_VALIDATORS_WEIGHT_BY_ERA_ID_NAME,
                GET_ERA_VALIDATORS_WEIGHT_BY_ERA_ID_HELP,
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

impl<REv: ReactorEventT> Component<REv> for ContractRuntime
where
    REv: From<Event> + Send,
{
    type Event = Event;
    type ConstructionError = ConfigError;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Request(request) => {
                match *request {
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
                        self.engine_state.flush_environment();
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
                            let result =
                                engine_state.commit_upgrade(correlation_id, *upgrade_config);
                            engine_state.flush_environment();
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
                            let era_validators =
                                engine_state.get_era_validators(correlation_id, request);
                            metrics
                                .get_validator_weights
                                .observe(start.elapsed().as_secs_f64());
                            trace!(?era_validators, "is validator bonded result");
                            let is_bonded = era_validators.and_then(|validator_map| {
                                match validator_map.get(&era_id) {
                                    None => Err(GetEraValidatorsError::EraValidatorsMissing),
                                    Some(era_validators) => {
                                        Ok(era_validators.contains_key(&validator_key))
                                    }
                                }
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
                    ContractRuntimeRequest::GetValidatorWeightsByEraId { request, responder } => {
                        trace!(?request, "get validator weights by era id request");
                        let engine_state = Arc::clone(&self.engine_state);
                        let metrics = Arc::clone(&self.metrics);
                        // Increment the counter to track the amount of times
                        // GetEraValidatorsByEraId was requested.
                        async move {
                            let correlation_id = CorrelationId::new();
                            let start = Instant::now();
                            let era_id = request.era_id();
                            let era_validators =
                                engine_state.get_era_validators(correlation_id, request.into());
                            let result: Result<Option<ValidatorWeights>, GetEraValidatorsError> =
                                match era_validators {
                                    Ok(era_validators) => {
                                        let validator_weights =
                                            era_validators.get(&era_id).cloned();
                                        Ok(validator_weights)
                                    }
                                    Err(GetEraValidatorsError::EraValidatorsMissing) => Ok(None),
                                    Err(error) => Err(error),
                                };
                            metrics
                                .get_era_validator_weights_by_era_id
                                .observe(start.elapsed().as_secs_f64());
                            trace!(?result, "get validator weights by era id response");
                            responder.respond(result).await
                        }
                        .ignore()
                    }
                    ContractRuntimeRequest::Step {
                        step_request,
                        responder,
                    } => {
                        trace!(?step_request, "step request");
                        let engine_state = Arc::clone(&self.engine_state);
                        let metrics = Arc::clone(&self.metrics);
                        async move {
                            let correlation_id = CorrelationId::new();
                            let start = Instant::now();
                            let result = engine_state.commit_step(correlation_id, step_request);
                            engine_state.flush_environment();
                            metrics.commit_step.observe(start.elapsed().as_secs_f64());
                            trace!(?result, "step response");
                            responder.respond(result).await
                        }
                        .ignore()
                    }
                    ContractRuntimeRequest::ReadTrie {
                        trie_key,
                        responder,
                    } => {
                        trace!(?trie_key, "read_trie request");
                        let engine_state = Arc::clone(&self.engine_state);
                        let metrics = Arc::clone(&self.metrics);
                        async move {
                            let correlation_id = CorrelationId::new();
                            let start = Instant::now();
                            let result = engine_state.read_trie(correlation_id, trie_key);
                            metrics.read_trie.observe(start.elapsed().as_secs_f64());
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
                                .put_trie_and_find_missing_descendant_trie_keys(
                                    correlation_id,
                                    &*trie,
                                );
                            engine_state.flush_environment();
                            metrics.put_trie.observe(start.elapsed().as_secs_f64());
                            trace!(?result, "put_trie response");
                            responder.respond(result).await
                        }
                        .ignore()
                    }
                    ContractRuntimeRequest::ExecuteBlock(finalized_block) => {
                        debug!(?finalized_block, "execute block");
                        effect_builder
                            .get_block_at_height_local(finalized_block.height())
                            .event(move |maybe_block| {
                                maybe_block.map(Box::new).map_or_else(
                                    || Event::BlockIsNew(Box::new(finalized_block)),
                                    Event::BlockAlreadyExists,
                                )
                            })
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
                            let result =
                                engine_state.missing_trie_keys(correlation_id, vec![trie_key]);
                            metrics.read_trie.observe(start.elapsed().as_secs_f64());
                            trace!(?result, "missing_trie_keys response");
                            responder.respond(result).await
                        }
                        .ignore()
                    }
                }
            }
            Event::BlockAlreadyExists(block) => effect_builder
                .announce_block_already_executed(*block)
                .ignore(),
            // If we haven't executed the block before in the past (for example during
            // joining), do it now.
            Event::BlockIsNew(finalized_block) => {
                self.get_deploys(effect_builder, *finalized_block)
            }
            Event::Result(contract_runtime_result) => match *contract_runtime_result {
                ContractRuntimeResult::GetDeploysResult {
                    finalized_block,
                    deploys,
                } => {
                    trace!(total = %deploys.len(), ?deploys, "fetched deploys");
                    self.handle_get_deploys_result(effect_builder, finalized_block, deploys)
                }

                ContractRuntimeResult::GetParentResult {
                    finalized_block,
                    deploys,
                    parent,
                } => {
                    trace!(parent_found = %parent.is_some(), finalized_height = %finalized_block.height(), "fetched parent");
                    let parent_summary = parent.map(|(hash, accumulated_seed, state_root_hash)| {
                        ExecutedBlockSummary {
                            hash,
                            state_root_hash,
                            accumulated_seed,
                        }
                    });
                    self.handle_get_parent_result(
                        effect_builder,
                        finalized_block,
                        deploys,
                        parent_summary,
                    )
                }
                ContractRuntimeResult::RunStepResult { mut state, result } => {
                    trace!(?result, "run step result");
                    match result {
                        Ok(StepResult::Success {
                            post_state_hash,
                            next_era_validators,
                            execution_effect,
                        }) => {
                            state.state_root_hash = post_state_hash.into();
                            let era_id = state.finalized_block.era_id();
                            let mut effects = effect_builder
                                .announce_step_success(era_id, execution_effect)
                                .ignore();
                            effects.extend(self.finalize_block_execution(
                                effect_builder,
                                state,
                                Some(next_era_validators),
                            ));
                            effects
                        }
                        _ => {
                            // When step fails, the auction process is broken and we should panic.
                            error!(?result, "run step failed - internal contract runtime error");
                            panic!("unable to run step");
                        }
                    }
                }
                ContractRuntimeResult::ExecutedAndCommitted(state) => {
                    self.execute_all_deploys_or_finalize_block_or_step(effect_builder, state)
                }
            },
        }
    }
}

/// Error returned from mis-configuring the contract runtime component.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Error initializing the LMDB environment.
    #[error("failed to initialize LMDB environment for contract runtime: {0}")]
    Lmdb(#[from] StorageLmdbError),
    /// Error initializing metrics.
    #[error("failed to initialize metrics for contract runtime: {0}")]
    Prometheus(#[from] prometheus::Error),
}

impl ContractRuntime {
    pub(crate) fn new(
        initial_state_root_hash: Digest,
        initial_block_header: Option<&BlockHeader>,
        protocol_version: ProtocolVersion,
        storage_config: WithDir<StorageConfig>,
        contract_runtime_config: &Config,
        registry: &Registry,
    ) -> Result<Self, ConfigError> {
        let initial_state = InitialState::new(initial_state_root_hash, initial_block_header);
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
            initial_state,
            protocol_version,
            parent_map: HashMap::new(),
            exec_queue: HashMap::new(),
            engine_state,
            metrics,
        })
    }

    /// Commits a genesis using a chainspec
    fn commit_genesis(
        &self,
        chainspec: Arc<Chainspec>,
    ) -> Result<GenesisResult, engine_state::Error> {
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

    pub(crate) fn set_initial_state(
        &mut self,
        initial_state_root_hash: Digest,
        initial_block_header: Option<&BlockHeader>,
    ) {
        self.initial_state = InitialState::new(initial_state_root_hash, initial_block_header);
    }

    /// Adds the "parent map" to the instance of `ContractRuntime`.
    ///
    /// When transitioning from `joiner` to `validator` states we need
    /// to carry over the last finalized block so that the next blocks in the linear chain
    /// have the state to build on.
    pub(crate) fn set_parent_map_from_block(
        &mut self,
        maybe_last_finalized_block_header: Option<BlockHeader>,
    ) {
        let parent_map = maybe_last_finalized_block_header
            .into_iter()
            .map(|block_header| {
                (
                    block_header.height(),
                    ExecutedBlockSummary {
                        hash: block_header.hash(),
                        state_root_hash: *block_header.state_root_hash(),
                        accumulated_seed: block_header.accumulated_seed(),
                    },
                )
            })
            .collect();
        self.parent_map = parent_map;
    }

    /// Gets the deploy(s) of the given finalized block from storage.
    fn get_deploys<REv: ReactorEventT>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        finalized_block: FinalizedBlock,
    ) -> Effects<Event> {
        let deploy_hashes = finalized_block
            .deploys_and_transfers_iter()
            .map(DeployHash::from)
            .collect::<SmallVec<_>>();
        if deploy_hashes.is_empty() {
            let result_event = move |_| {
                Event::Result(Box::new(ContractRuntimeResult::GetDeploysResult {
                    finalized_block,
                    deploys: VecDeque::new(),
                }))
            };
            return effect_builder.immediately().event(result_event);
        }

        let era_id = finalized_block.era_id();
        let height = finalized_block.height();

        // Get all deploys in order they appear in the finalized block.
        effect_builder
            .get_deploys_from_storage(deploy_hashes)
            .event(move |result| {
                Event::Result(Box::new(ContractRuntimeResult::GetDeploysResult {
                    finalized_block,
                    deploys: result
                        .into_iter()
                        // Assumes all deploys are present
                        .map(|maybe_deploy| {
                            maybe_deploy.unwrap_or_else(|| {
                                panic!(
                                "deploy for block in era={} and height={} is expected to exist \
                                in the storage",
                                era_id, height
                            )
                            })
                        })
                        .collect(),
                }))
            })
    }

    /// Creates and announces the linear chain block.
    fn finalize_block_execution<REv: ReactorEventT>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        state: Box<RequestState>,
        next_era_validator_weights: Option<BTreeMap<PublicKey, U512>>,
    ) -> Effects<Event> {
        // The state hash of the last execute-commit cycle is used as the block's post state
        // hash.
        let next_height = state.finalized_block.height() + 1;
        // Update the metric.
        self.metrics
            .chain_height
            .set(state.finalized_block.height() as i64);
        let block = self.create_block(
            state.finalized_block,
            state.state_root_hash,
            next_era_validator_weights,
        );

        let mut effects = effect_builder
            .announce_linear_chain_block(block, state.execution_results)
            .ignore();
        // If the child is already finalized, start execution.
        if let Some((finalized_block, deploys)) = self.exec_queue.remove(&next_height) {
            effects.extend(self.handle_get_deploys_result(
                effect_builder,
                finalized_block,
                deploys,
            ));
        }
        effects
    }

    fn execute_all_deploys_or_finalize_block_or_step<REv: ReactorEventT>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        state: Box<RequestState>,
    ) -> Effects<Event> {
        if state.remaining_deploys.is_empty() {
            self.finalize_block_or_step(effect_builder, state)
        } else {
            self.execute_all_deploys_in_block(state)
        }
    }

    fn finalize_block_or_step<REv: ReactorEventT>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        state: Box<RequestState>,
    ) -> Effects<Event> {
        let era_end = match state.finalized_block.era_report() {
            Some(era_end) => era_end,
            // Not at a switch block, so we don't need to have next_era_validators when
            // constructing the next block
            None => return self.finalize_block_execution(effect_builder, state, None),
        };
        let reward_items = era_end
            .rewards
            .iter()
            .map(|(vid, &value)| RewardItem::new(vid.clone(), value))
            .collect();
        let slash_items = era_end
            .equivocators
            .iter()
            .map(|vid| SlashItem::new(vid.clone()))
            .collect();
        let evict_items = era_end
            .inactive_validators
            .iter()
            .map(|vid| EvictItem::new(vid.clone()))
            .collect();
        let era_end_timestamp_millis = state.finalized_block.timestamp().millis();
        let request = StepRequest {
            pre_state_hash: state.state_root_hash.into(),
            protocol_version: self.protocol_version,
            reward_items,
            slash_items,
            evict_items,
            run_auction: true,
            next_era_id: state.finalized_block.era_id().successor(),
            era_end_timestamp_millis,
        };
        effect_builder.run_step(request).event(|result| {
            Event::Result(Box::new(ContractRuntimeResult::RunStepResult {
                state,
                result,
            }))
        })
    }

    fn execute_all_deploys_in_block(&mut self, mut state: Box<RequestState>) -> Effects<Event> {
        let engine_state = Arc::clone(&self.engine_state);
        let metrics = Arc::clone(&self.metrics);
        let protocol_version = self.protocol_version;
        let block_time = state.finalized_block.timestamp().millis();
        let proposer = state.finalized_block.proposer();
        async move {
            for deploy in state.remaining_deploys.drain(..) {
                let deploy_hash = *deploy.id();
                let deploy_header = deploy.header().clone();
                let deploy_item = DeployItem::from(deploy);

                let execute_request = ExecuteRequest::new(
                    state.state_root_hash.into(),
                    block_time,
                    vec![deploy_item],
                    protocol_version,
                    proposer.clone(),
                );

                // TODO: this is currently working coincidentally because we are passing only one
                // deploy_item per exec. The execution results coming back from the ee lacks the
                // mapping between deploy_hash and execution result, and this outer logic is
                // enriching it with the deploy hash. If we were passing multiple deploys per exec
                // the relation between the deploy and the execution results would be lost.
                let result =
                    operations::execute(engine_state.clone(), metrics.clone(), execute_request)
                        .await;

                trace!(%deploy_hash, ?result, "deploy execution result");
                // As for now a given state is expected to exist.
                let execution_results = result.unwrap();
                match operations::commit_execution_effects(
                    engine_state.clone(),
                    metrics.clone(),
                    state.state_root_hash,
                    deploy_hash,
                    execution_results,
                )
                .await
                {
                    Ok((state_hash, execution_result)) => {
                        state
                            .execution_results
                            .insert(deploy_hash, (deploy_header, execution_result));
                        state.state_root_hash = state_hash;
                    }
                    // When commit fails we panic as we'll not be able to execute the next
                    // block.
                    Err(_err) => panic!("unable to commit"),
                }
            }
            engine_state.flush_environment();
            state
        }
        .event(|state| Event::Result(Box::new(ContractRuntimeResult::ExecutedAndCommitted(state))))
    }

    fn handle_get_deploys_result<REv: ReactorEventT>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        finalized_block: FinalizedBlock,
        deploys: VecDeque<Deploy>,
    ) -> Effects<Event> {
        if let Some(state_root_hash) = self.pre_state_hash(&finalized_block) {
            let state = Box::new(RequestState {
                finalized_block,
                remaining_deploys: deploys,
                execution_results: HashMap::new(),
                state_root_hash,
            });
            self.execute_all_deploys_or_finalize_block_or_step(effect_builder, state)
        } else {
            // Didn't find parent in the `parent_map` cache.
            // Read it from the storage.
            let height = finalized_block.height();
            effect_builder
                .get_block_at_height_local(height - 1)
                .event(|parent| {
                    Event::Result(Box::new(ContractRuntimeResult::GetParentResult {
                        finalized_block,
                        deploys,
                        parent: parent.map(|b| {
                            (
                                *b.hash(),
                                b.header().accumulated_seed(),
                                *b.state_root_hash(),
                            )
                        }),
                    }))
                })
        }
    }

    fn handle_get_parent_result<REv: ReactorEventT>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        finalized_block: FinalizedBlock,
        deploys: VecDeque<Deploy>,
        parent: Option<ExecutedBlockSummary>,
    ) -> Effects<Event> {
        match parent {
            None => {
                let height = finalized_block.height();
                debug!("no pre-state hash for height {}", height);
                // re-check the parent map - the parent might have been executed in the meantime!
                if let Some(state_root_hash) = self.pre_state_hash(&finalized_block) {
                    let state = Box::new(RequestState {
                        finalized_block,
                        remaining_deploys: deploys,
                        execution_results: HashMap::new(),
                        state_root_hash,
                    });
                    self.execute_all_deploys_or_finalize_block_or_step(effect_builder, state)
                } else {
                    // The parent block has not been executed yet; delay handling.
                    self.exec_queue.insert(height, (finalized_block, deploys));
                    Effects::new()
                }
            }
            Some(parent_summary) => {
                // Parent found in the storage.
                // Insert into `parent_map` cache.
                // It will be removed in `create_block` method.
                self.parent_map
                    .insert(finalized_block.height().saturating_sub(1), parent_summary);
                self.handle_get_deploys_result(effect_builder, finalized_block, deploys)
            }
        }
    }

    fn create_block(
        &mut self,
        finalized_block: FinalizedBlock,
        state_root_hash: Digest,
        next_era_validator_weights: Option<BTreeMap<PublicKey, U512>>,
    ) -> Block {
        let (parent_summary_hash, parent_seed) = if self.is_initial_block_child(&finalized_block) {
            // The first block after the initial one: get initial block summary if we have one, or
            // if not, this should be the genesis child and so we take the default values.
            (
                self.initial_state
                    .block_summary
                    .as_ref()
                    .map(|summary| summary.hash)
                    .unwrap_or_else(|| BlockHash::new(Digest::default())),
                self.initial_state
                    .block_summary
                    .as_ref()
                    .map(|summary| summary.accumulated_seed)
                    .unwrap_or_default(),
            )
        } else {
            let parent_block_height = finalized_block.height() - 1;
            let summary = self
                .parent_map
                .remove(&parent_block_height)
                .unwrap_or_else(|| panic!("failed to take {:?}", parent_block_height));
            (summary.hash, summary.accumulated_seed)
        };
        let block_height = finalized_block.height();
        let block = Block::new(
            parent_summary_hash,
            parent_seed,
            state_root_hash,
            finalized_block,
            next_era_validator_weights,
            self.protocol_version,
        );
        let summary = ExecutedBlockSummary {
            hash: *block.hash(),
            state_root_hash,
            accumulated_seed: block.header().accumulated_seed(),
        };
        let _ = self.parent_map.insert(block_height, summary);
        block
    }

    fn pre_state_hash(&mut self, finalized_block: &FinalizedBlock) -> Option<Digest> {
        if self.is_initial_block_child(finalized_block) {
            Some(self.initial_state.state_root_hash)
        } else {
            // Try to get the parent's post-state-hash from the `parent_map`.
            // We're subtracting 1 from the height as we want to get _parent's_ post-state hash.
            let parent_block_height = finalized_block.height() - 1;
            self.parent_map
                .get(&parent_block_height)
                .map(|summary| summary.state_root_hash)
        }
    }

    /// Returns true if the `finalized_block` is an immediate child of the initial block, ie.
    /// either genesis or the highest known block at the time of initializing the component.
    fn is_initial_block_child(&self, finalized_block: &FinalizedBlock) -> bool {
        finalized_block.height() == self.initial_state.child_height
    }
}

/// Holds the state of an ongoing execute-commit cycle spawned from a given `Event::Request`.
#[derive(Debug)]
pub struct RequestState {
    /// Finalized block for this request.
    pub finalized_block: FinalizedBlock,
    /// Deploys which have still to be executed.
    pub remaining_deploys: VecDeque<Deploy>,
    /// A collection of results of executing the deploys.
    pub execution_results: HashMap<DeployHash, (DeployHeader, ExecutionResult)>,
    /// Current state root hash of global storage.  Is initialized with the parent block's
    /// state hash, and is updated after each commit.
    pub state_root_hash: Digest,
}

#[derive(DataSize, Debug, Default)]
struct InitialState {
    /// Height of the child of the highest known block at the time of initializing the component.
    /// Required for the block executor to know when to stop looking for parent blocks when getting
    /// the pre-state hash for execution. With upgrades, we could get a wrong hash if we went too
    /// far.
    child_height: u64,
    /// Summary of the highest known block.
    block_summary: Option<ExecutedBlockSummary>,
    /// Initial state root hash.
    state_root_hash: Digest,
}

impl InitialState {
    fn new(state_root_hash: Digest, block_header: Option<&BlockHeader>) -> Self {
        let block_summary = block_header.map(|hdr| ExecutedBlockSummary {
            hash: hdr.hash(),
            state_root_hash,
            accumulated_seed: hdr.accumulated_seed(),
        });
        Self {
            child_height: block_header.map_or(0, |hdr| hdr.height() + 1),
            block_summary,
            state_root_hash,
        }
    }
}
