mod event;
mod metrics;
mod pending_signatures;
mod signature;
mod signature_cache;
mod state;

use datasize::DataSize;
use std::{convert::Infallible, fmt::Display, marker::PhantomData};

use itertools::Itertools;
use prometheus::Registry;
use tracing::{debug, error};

use self::{
    metrics::LinearChainMetrics,
    state::{Outcome, Outcomes},
};
use super::Component;
use crate::{
    effect::{
        announcements::LinearChainAnnouncement,
        requests::{
            ChainspecLoaderRequest, ContractRuntimeRequest, LinearChainRequest, NetworkRequest,
            StorageRequest,
        },
        EffectBuilder, EffectExt, EffectResultExt, Effects,
    },
    protocol::Message,
    types::BlockByHeight,
    NodeRng,
};
use casper_types::ProtocolVersion;

pub use event::Event;
use state::LinearChain;

#[derive(DataSize, Debug)]
pub(crate) struct LinearChainComponent<I> {
    linear_chain_state: LinearChain,
    #[data_size(skip)]
    metrics: LinearChainMetrics,
    _marker: PhantomData<I>,
}

impl<I> LinearChainComponent<I> {
    pub(crate) fn new(
        registry: &Registry,
        protocol_version: ProtocolVersion,
        auction_delay: u64,
        unbonding_delay: u64,
    ) -> Result<Self, prometheus::Error> {
        let metrics = LinearChainMetrics::new(registry)?;
        let linear_chain_state = LinearChain::new(protocol_version, auction_delay, unbonding_delay);
        Ok(LinearChainComponent {
            linear_chain_state,
            metrics,
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
                match effect_builder.get_block_from_storage(block_hash).await {
                    None => debug!("failed to get {} for {}", block_hash, sender),
                    Some(block) => match Message::new_get_response(&block) {
                        Ok(message) => effect_builder.send_message(sender, message).await,
                        Err(error) => error!("failed to create get-response {}", error),
                    },
                }
            }
            .ignore(),
            Event::Request(LinearChainRequest::BlockAtHeight(height, sender)) => async move {
                let block_by_height = match effect_builder
                    .get_block_at_height_from_storage(height)
                    .await
                {
                    None => {
                        debug!("failed to get {} for {}", height, sender);
                        BlockByHeight::Absent(height)
                    }
                    Some(block) => BlockByHeight::new(block),
                };
                match Message::new_get_response(&block_by_height) {
                    Ok(message) => effect_builder.send_message(sender, message).await,
                    Err(error) => {
                        error!("failed to create get-response {}", error);
                    }
                }
            }
            .ignore(),
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
