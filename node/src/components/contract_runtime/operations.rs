use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};

use engine_state::ExecuteRequest;
use itertools::Itertools;
use tracing::{debug, trace};

use casper_execution_engine::{
    core::engine_state::{
        self, step::EvictItem, DeployItem, EngineState, ExecutionResult as EngineExecutionResult,
        ExecutionResults, RewardItem, StepError, StepRequest, StepSuccess,
    },
    shared::{additive_map::AdditiveMap, newtypes::CorrelationId, transform::Transform},
    storage::global_state::lmdb::LmdbGlobalState,
};
use casper_types::{EraId, ExecutionResult, Key, ProtocolVersion, PublicKey};

use crate::{
    components::{
        consensus::EraReport,
        contract_runtime::{
            error::BlockExecutionError, BlockAndExecutionEffects, ContractRuntimeMetrics,
            ExecutionPreState,
        },
    },
    crypto::hash::Digest,
    types::{Block, Deploy, DeployHash, DeployHeader, FinalizedBlock},
};

pub(super) fn execute_finalized_block(
    engine_state: &EngineState<LmdbGlobalState>,
    metrics: &ContractRuntimeMetrics,
    protocol_version: ProtocolVersion,
    execution_pre_state: ExecutionPreState,
    finalized_block: FinalizedBlock,
    deploys: Vec<Deploy>,
) -> Result<BlockAndExecutionEffects, BlockExecutionError> {
    if finalized_block.height() != execution_pre_state.next_block_height {
        return Err(BlockExecutionError::WrongBlockHeight {
            finalized_block: Box::new(finalized_block),
            execution_pre_state: Box::new(execution_pre_state),
        });
    }
    let ExecutionPreState {
        pre_state_root_hash,
        parent_hash,
        parent_seed,
        next_block_height: _,
    } = execution_pre_state;
    let mut state_root_hash = pre_state_root_hash;
    let mut execution_results: HashMap<DeployHash, (DeployHeader, ExecutionResult)> =
        HashMap::new();
    // Run any deploys that must be executed
    let block_time = finalized_block.timestamp().millis();
    for deploy in deploys {
        let deploy_hash = *deploy.id();
        let deploy_header = deploy.header().clone();
        let execute_request = ExecuteRequest::new(
            state_root_hash.into(),
            block_time,
            vec![DeployItem::from(deploy)],
            protocol_version,
            finalized_block.proposer().clone(),
        );

        // TODO: this is currently working coincidentally because we are passing only one
        // deploy_item per exec. The execution results coming back from the ee lacks the
        // mapping between deploy_hash and execution result, and this outer logic is
        // enriching it with the deploy hash. If we were passing multiple deploys per exec
        // the relation between the deploy and the execution results would be lost.
        let result = execute(engine_state, metrics, execute_request)?;

        trace!(?deploy_hash, ?result, "deploy execution result");
        // As for now a given state is expected to exist.
        let (state_hash, execution_result) =
            commit_execution_effects(engine_state, metrics, state_root_hash, deploy_hash, result)?;
        execution_results.insert(deploy_hash, (deploy_header, execution_result));
        state_root_hash = state_hash;
    }

    // If the finalized block has an era report, run the auction contract
    let maybe_step_success = match finalized_block.era_report() {
        None => None,
        Some(era_report) => Some(commit_step(
            engine_state,
            metrics,
            protocol_version,
            state_root_hash,
            era_report,
            finalized_block.timestamp().millis(),
            finalized_block.era_id().successor(),
        )?),
    };

    // Update the metric.
    let block_height = finalized_block.height();
    metrics.chain_height.set(block_height as i64);

    let block_and_execution_effects = if let Some(StepSuccess {
        post_state_hash,
        next_era_validators,
        execution_effect,
    }) = maybe_step_success
    {
        BlockAndExecutionEffects {
            block: Block::new(
                parent_hash,
                parent_seed,
                post_state_hash.into(),
                finalized_block,
                Some(next_era_validators),
                protocol_version,
            )?,
            execution_results,
            maybe_step_execution_effect: Some(execution_effect),
        }
    } else {
        BlockAndExecutionEffects {
            block: Block::new(
                parent_hash,
                parent_seed,
                state_root_hash,
                finalized_block,
                None,
                protocol_version,
            )?,
            execution_results,
            maybe_step_execution_effect: None,
        }
    };
    Ok(block_and_execution_effects)
}

/// Commits the execution effects.
fn commit_execution_effects(
    engine_state: &EngineState<LmdbGlobalState>,
    metrics: &ContractRuntimeMetrics,
    state_root_hash: Digest,
    deploy_hash: DeployHash,
    execution_results: ExecutionResults,
) -> Result<(Digest, ExecutionResult), BlockExecutionError> {
    let ee_execution_result = execution_results
        .into_iter()
        .exactly_one()
        .map_err(|_| BlockExecutionError::MoreThanOneExecutionResult)?;
    let execution_result = ExecutionResult::from(&ee_execution_result);

    let execution_effect = match ee_execution_result {
        EngineExecutionResult::Success {
            execution_effect,
            cost,
            ..
        } => {
            // We do want to see the deploy hash and cost in the logs.
            // We don't need to see the effects in the logs.
            debug!(?deploy_hash, %cost, "execution succeeded");
            execution_effect
        }
        EngineExecutionResult::Failure {
            error,
            execution_effect,
            cost,
            ..
        } => {
            // Failure to execute a contract is a user error, not a system error.
            // We do want to see the deploy hash, error, and cost in the logs.
            // We don't need to see the effects in the logs.
            debug!(?deploy_hash, ?error, %cost, "execution failure");
            execution_effect
        }
    };
    let new_state_root = commit_transforms(
        engine_state,
        metrics,
        state_root_hash,
        execution_effect.transforms,
    )?;
    Ok((new_state_root, execution_result))
}

fn commit_transforms(
    engine_state: &EngineState<LmdbGlobalState>,
    metrics: &ContractRuntimeMetrics,
    state_root_hash: Digest,
    effects: AdditiveMap<Key, Transform>,
) -> Result<Digest, engine_state::Error> {
    trace!(?state_root_hash, ?effects, "commit");
    let correlation_id = CorrelationId::new();
    let start = Instant::now();
    let result = engine_state.apply_effect(correlation_id, state_root_hash.into(), effects);
    metrics.apply_effect.observe(start.elapsed().as_secs_f64());
    trace!(?result, "commit result");
    result.map(Digest::from)
}

fn execute(
    engine_state: &EngineState<LmdbGlobalState>,
    metrics: &ContractRuntimeMetrics,
    execute_request: ExecuteRequest,
) -> Result<VecDeque<EngineExecutionResult>, engine_state::Error> {
    trace!(?execute_request, "execute");
    let correlation_id = CorrelationId::new();
    let start = Instant::now();
    let result = engine_state.run_execute(correlation_id, execute_request);
    metrics.run_execute.observe(start.elapsed().as_secs_f64());
    trace!(?result, "execute result");
    result
}

fn commit_step(
    engine_state: &EngineState<LmdbGlobalState>,
    metrics: &ContractRuntimeMetrics,
    protocol_version: ProtocolVersion,
    pre_state_root_hash: Digest,
    era_report: &EraReport<PublicKey>,
    era_end_timestamp_millis: u64,
    next_era_id: EraId,
) -> Result<StepSuccess, StepError> {
    // Extract the rewards and the inactive validators if this is a switch block
    let EraReport {
        equivocators,
        rewards,
        inactive_validators,
    } = era_report;

    let reward_items = rewards
        .clone()
        .into_iter()
        .map(|(vid, value)| RewardItem::new(vid, value))
        .collect();

    // Both inactive validators and equivocators are evicted
    let evict_items = inactive_validators
        .iter()
        .chain(equivocators)
        .cloned()
        .map(EvictItem::new)
        .collect();

    let step_request = StepRequest {
        pre_state_hash: pre_state_root_hash.into(),
        protocol_version,
        reward_items,
        // Note: The Casper Network does not slash, but another network could
        slash_items: vec![],
        evict_items,
        run_auction: true,
        next_era_id,
        era_end_timestamp_millis,
    };

    // Have the EE commit the step.
    let correlation_id = CorrelationId::new();
    let start = Instant::now();
    let result = engine_state.commit_step(correlation_id, step_request);
    metrics.commit_step.observe(start.elapsed().as_secs_f64());
    trace!(?result, "step response");
    result
}
