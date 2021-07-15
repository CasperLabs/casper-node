mod event;
mod metrics;
mod pending_signatures;
mod signature;
mod signature_cache;
mod state;

use datasize::DataSize;
use std::{convert::Infallible, fmt::Display, marker::PhantomData, sync::Arc};

use itertools::Itertools;
use prometheus::Registry;
use tracing::error;

use casper_types::ProtocolVersion;

use crate::{
    components::{
        fetcher::FetchedOrNotFound,
        linear_chain::state::{Outcome, Outcomes},
        Component,
    },
    effect::{
        announcements::LinearChainAnnouncement,
        requests::{
            ChainspecLoaderRequest, ContractRuntimeRequest, LinearChainRequest, NetworkRequest,
            StorageRequest,
        },
        EffectBuilder, EffectExt, EffectResultExt, Effects,
    },
    protocol::Message,
    types::Chainspec,
    NodeRng,
};

pub use event::Event;
use metrics::LinearChainMetrics;
use state::LinearChain;

#[derive(DataSize, Debug)]
pub(crate) struct LinearChainComponent<I> {
    linear_chain_state: LinearChain,
    #[data_size(skip)]
    metrics: LinearChainMetrics,
    chainspec: Arc<Chainspec>,
    _marker: PhantomData<I>,
}

impl<I> LinearChainComponent<I> {
    pub(crate) fn new(
        registry: &Registry,
        protocol_version: ProtocolVersion,
        chainspec: Arc<Chainspec>,
    ) -> Result<Self, prometheus::Error> {
        let metrics = LinearChainMetrics::new(registry)?;
        let auction_delay = chainspec.core_config.auction_delay;
        let unbonding_delay = chainspec.core_config.unbonding_delay;
        let linear_chain_state = LinearChain::new(protocol_version, auction_delay, unbonding_delay);
        Ok(LinearChainComponent {
            linear_chain_state,
            metrics,
            chainspec,
            _marker: PhantomData,
        })
    }
}

fn outcomes_to_effects<REv, I>(
    effect_builder: EffectBuilder<REv>,
    outcomes: Outcomes,
) -> Effects<Event<I>>
where
    REv: From<StorageRequest>
        + From<NetworkRequest<I, Message>>
        + From<LinearChainAnnouncement>
        + From<ContractRuntimeRequest>
        + From<ChainspecLoaderRequest>
        + Send,
    I: Display + Send + 'static,
{
    outcomes
        .into_iter()
        .map(|outcome| match outcome {
            Outcome::StoreBlockSignatures(block_signatures) => effect_builder
                .put_signatures_to_storage(block_signatures)
                .ignore(),
            Outcome::StoreExecutionResults(block_hash, execution_results) => effect_builder
                .put_execution_results_to_storage(block_hash, execution_results)
                .ignore(),
            Outcome::StoreBlock(block) => effect_builder
                .put_block_to_storage(block.clone())
                .event(move |_| Event::PutBlockResult { block }),
            Outcome::Gossip(fs) => {
                let message = Message::FinalitySignature(fs);
                effect_builder.broadcast_message(message).ignore()
            }
            Outcome::AnnounceSignature(fs) => {
                effect_builder.announce_finality_signature(fs).ignore()
            }
            Outcome::AnnounceBlock(block) => effect_builder.announce_block_added(block).ignore(),
            Outcome::LoadSignatures(fs) => effect_builder
                .get_signatures_from_storage(fs.block_hash)
                .event(move |maybe_signatures| {
                    Event::GetStoredFinalitySignaturesResult(fs, maybe_signatures.map(Box::new))
                }),
            Outcome::VerifyIfBonded {
                new_fs,
                known_fs,
                protocol_version,
                latest_state_root_hash,
            } => effect_builder
                .is_bonded_validator(
                    new_fs.public_key.clone(),
                    new_fs.era_id,
                    latest_state_root_hash,
                    protocol_version,
                )
                .result(
                    |is_bonded| Event::IsBonded(known_fs, new_fs, is_bonded),
                    |error| {
                        error!(%error, "checking in future eras returned an error.");
                        panic!("couldn't check if validator is bonded")
                    },
                ),
        })
        .concat()
}

impl<I, REv> Component<REv> for LinearChainComponent<I>
where
    REv: From<StorageRequest>
        + From<NetworkRequest<I, Message>>
        + From<LinearChainAnnouncement>
        + From<ContractRuntimeRequest>
        + From<ChainspecLoaderRequest>
        + Send,
    I: Display + Send + 'static,
{
    type Event = Event<I>;
    type ConstructionError = Infallible;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Request(LinearChainRequest::BlockRequest(block_hash, sender)) => async move {
                let fetched_or_not_found_block =
                    match effect_builder.get_block_from_storage(block_hash).await {
                        None => FetchedOrNotFound::NotFound(block_hash),
                        Some(block) => FetchedOrNotFound::Fetched(block),
                    };
                match Message::new_get_response(&fetched_or_not_found_block) {
                    Ok(message) => effect_builder.send_message(sender, message).await,
                    Err(error) => error!("failed to create get-response {}", error),
                }
            }
            .ignore(),
            Event::Request(LinearChainRequest::BlockWithMetadataAtHeight(height, sender)) => {
                handle_block_with_metadata_at_height_request(
                    self.chainspec.clone(),
                    effect_builder,
                    height,
                    sender,
                )
                .ignore()
            }
            Event::NewLinearChainBlock {
                block,
                execution_results,
            } => {
                let outcomes = self
                    .linear_chain_state
                    .handle_new_block(block, execution_results);
                outcomes_to_effects(effect_builder, outcomes)
            }
            Event::PutBlockResult { block } => {
                let completion_duration = block.header().timestamp().elapsed().millis();
                self.metrics
                    .block_completion_duration
                    .set(completion_duration as i64);
                let outcomes = self.linear_chain_state.handle_put_block(block);
                outcomes_to_effects(effect_builder, outcomes)
            }
            Event::FinalitySignatureReceived(fs, gossiped) => {
                let outcomes = self
                    .linear_chain_state
                    .handle_finality_signature(fs, gossiped);
                outcomes_to_effects(effect_builder, outcomes)
            }
            Event::GetStoredFinalitySignaturesResult(fs, maybe_signatures) => {
                let outcomes = self
                    .linear_chain_state
                    .handle_cached_signatures(maybe_signatures, fs);
                outcomes_to_effects(effect_builder, outcomes)
            }
            Event::IsBonded(maybe_known_signatures, new_fs, is_bonded) => {
                let outcomes = self.linear_chain_state.handle_is_bonded(
                    maybe_known_signatures,
                    new_fs,
                    is_bonded,
                );
                outcomes_to_effects(effect_builder, outcomes)
            }
            Event::KnownLinearChainBlock(block) => {
                self.linear_chain_state.set_latest_block(*block);
                Effects::new()
            }
        }
    }
}

/// Handles a request for a block with finality signatures. Sends them to the peer only if we
/// have a block in storage at the specified height, and the finality signatures' total weight
/// exceeds the configured finality threshold.
async fn handle_block_with_metadata_at_height_request<REv, I>(
    chainspec: Arc<Chainspec>,
    effect_builder: EffectBuilder<REv>,
    height: u64,
    sender: I,
) where
    REv: From<StorageRequest>
        + From<NetworkRequest<I, Message>>
        + From<ContractRuntimeRequest>
        + From<ChainspecLoaderRequest>
        + Send,
    I: Display + Send + 'static,
{
    let maybe_block_with_metadata = effect_builder
        .get_block_at_height_with_metadata_from_storage(height)
        .await;
    let mut fully_signed = false;
    if let Some(block_with_metadata) = &maybe_block_with_metadata {
        if let Some(validator_weights) = effect_builder
            .get_era_validators(block_with_metadata.block.header().era_id())
            .await
        {
            fully_signed = super::linear_chain_sync::weigh_finality_signatures(
                &validator_weights,
                chainspec.highway_config.finality_threshold_fraction,
                &block_with_metadata.finality_signatures,
            )
            .is_ok();
        }
    }
    let fetch_or_not_found_block_with_metadata =
        match maybe_block_with_metadata.filter(|_| fully_signed) {
            None => FetchedOrNotFound::NotFound(height),
            Some(block) => FetchedOrNotFound::Fetched(block),
        };
    match Message::new_get_response(&fetch_or_not_found_block_with_metadata) {
        Ok(message) => effect_builder.send_message(sender, message).await,
        Err(error) => {
            error!("failed to create get-response {}", error);
        }
    }
}
