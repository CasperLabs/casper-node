mod event;
mod metrics;
mod pending_signatures;
mod signature;
mod signature_cache;
mod state;

use datasize::DataSize;
use std::{convert::Infallible, fmt::Display, marker::PhantomData};

use prometheus::Registry;
use tracing::{debug, error, info, warn};

use self::metrics::LinearChainMetrics;
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
    types::{BlockByHeight, BlockSignatures, FinalitySignature, Timestamp},
    NodeRng,
};
use casper_types::{EraId, ProtocolVersion};

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
        activation_era_id: EraId,
    ) -> Result<Self, prometheus::Error> {
        let metrics = LinearChainMetrics::new(registry)?;
        let linear_chain_state = LinearChain::new(
            protocol_version,
            auction_delay,
            unbonding_delay,
            activation_era_id,
        );
        Ok(LinearChainComponent {
            linear_chain_state,
            metrics,
            _marker: PhantomData,
        })
    }
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
            Event::Request(LinearChainRequest::BlockWithMetadataAtHeight(height, sender)) => {
                async move {
                    if let Some(block_with_metadata) = effect_builder
                        .get_block_at_height_with_metadata_from_storage(height)
                        .await
                    {
                        match Message::new_get_response(&block_with_metadata) {
                            Ok(message) => effect_builder.send_message(sender, message).await,
                            Err(error) => {
                                error!("failed to create get-response {}", error);
                            }
                        }
                    } else {
                        debug!("failed to get {} for {}", height, sender);
                    };
                }
                .ignore()
            }
            Event::NewLinearChainBlock {
                block,
                execution_results,
            } => {
                let signatures = self.linear_chain_state.new_block(&*block);
                let mut effects = Effects::new();
                if !signatures.is_empty() {
                    let mut block_signatures =
                        BlockSignatures::new(*block.hash(), block.header().era_id());
                    for sig in signatures.iter() {
                        block_signatures.insert_proof(sig.public_key(), sig.signature());
                    }
                    effects.extend(
                        effect_builder
                            .put_signatures_to_storage(block_signatures)
                            .ignore(),
                    );
                    for signature in signatures {
                        if signature.is_local() {
                            let message =
                                Message::FinalitySignature(Box::new(signature.to_inner().clone()));
                            effects.extend(effect_builder.broadcast_message(message).ignore());
                        }
                        effects.extend(
                            effect_builder
                                .announce_finality_signature(signature.take())
                                .ignore(),
                        );
                    }
                };
                let block_to_store = block.clone();
                effects.extend(
                    async move {
                        let block_hash = *block_to_store.hash();
                        effect_builder.put_block_to_storage(block_to_store).await;
                        effect_builder
                            .put_execution_results_to_storage(block_hash, execution_results)
                            .await;
                    }
                    .event(move |_| Event::PutBlockResult { block }),
                );
                effects
            }
            Event::PutBlockResult { block } => {
                self.linear_chain_state.set_latest_block(*block.clone());

                let completion_duration =
                    Timestamp::now().millis() - block.header().timestamp().millis();
                self.metrics
                    .block_completion_duration
                    .set(completion_duration as i64);

                let block_hash = block.header().hash();
                let era_id = block.header().era_id();
                let height = block.header().height();
                info!(%block_hash, %era_id, %height, "linear chain block stored");
                effect_builder.announce_block_added(block).ignore()
            }
            Event::FinalitySignatureReceived(fs, gossiped) => {
                let FinalitySignature { block_hash, .. } = *fs;
                if !self
                    .linear_chain_state
                    .add_pending_finality_signature(*fs.clone(), gossiped)
                {
                    // If we did not add the signature it means it's either incorrect or we already
                    // know it.
                    return Effects::new();
                }
                match self.linear_chain_state.get_signatures(&block_hash) {
                    // Not found in the cache, look in the storage.
                    None => effect_builder
                        .get_signatures_from_storage(block_hash)
                        .event(move |maybe_signatures| {
                            Event::GetStoredFinalitySignaturesResult(
                                fs,
                                maybe_signatures.map(Box::new),
                            )
                        }),
                    Some(signatures) => effect_builder.immediately().event(move |_| {
                        Event::GetStoredFinalitySignaturesResult(fs, Some(Box::new(signatures)))
                    }),
                }
            }
            Event::GetStoredFinalitySignaturesResult(fs, maybe_signatures) => {
                if let Some(known_signatures) = &maybe_signatures {
                    // If the newly-received finality signature does not match the era of previously
                    // validated signatures reject it as they can't both be
                    // correct – block was created in a specific era so the IDs have to match.
                    if known_signatures.era_id != fs.era_id {
                        warn!(public_key = %fs.public_key,
                            expected = %known_signatures.era_id,
                            got = %fs.era_id,
                            "finality signature with invalid era id.");
                        self.linear_chain_state.remove_from_pending_fs(&*fs);
                        // TODO: Disconnect from the sender.
                        return Effects::new();
                    }
                    if known_signatures.has_proof(&fs.public_key) {
                        self.linear_chain_state.remove_from_pending_fs(&fs);
                        return Effects::new();
                    }
                    // Populate cache so that next incoming signatures don't trigger read from the
                    // storage. If `known_signatures` are already from cache then this will be a
                    // noop.
                    self.linear_chain_state
                        .cache_signatures(*known_signatures.clone());
                }
                // Check if the validator is bonded in the era in which the block was created.
                // TODO: Use protocol version that is valid for the block's height.
                let protocol_version = self.linear_chain_state.current_protocol_version();
                let latest_state_root_hash = self
                    .linear_chain_state
                    .latest_block()
                    .as_ref()
                    .map(|block| *block.header().state_root_hash());
                effect_builder
                    .is_bonded_validator(
                        fs.public_key,
                        fs.era_id,
                        latest_state_root_hash,
                        protocol_version,
                    )
                    .result(
                        |is_bonded| Event::IsBonded(maybe_signatures, fs, is_bonded),
                        |error| {
                            error!(%error, "checking in future eras returned an error.");
                            panic!("couldn't check if validator is bonded")
                        },
                    )
            }
            Event::IsBonded(Some(mut known_signatures), fs, true) => {
                // New finality signature from a bonded validator.
                known_signatures.insert_proof(fs.public_key, fs.signature);
                // Cache the results in case we receive the same finality signature before we
                // manage to store it in the database.
                self.linear_chain_state
                    .cache_signatures(*known_signatures.clone());
                debug!(hash = %known_signatures.block_hash, "storing finality signatures");
                // Announce new finality signatures for other components to pick up.
                let mut effects = effect_builder
                    .announce_finality_signature(fs.clone())
                    .ignore();
                if let Some(signature) = self.linear_chain_state.remove_from_pending_fs(&*fs) {
                    // This shouldn't return `None` as we added the `fs` to the pending collection
                    // when we received it. If it _is_ `None` then a concurrent
                    // flow must have already removed it. If it's a signature
                    // created by this node, gossip it.
                    if signature.is_local() {
                        let message = Message::FinalitySignature(fs.clone());
                        effects.extend(effect_builder.broadcast_message(message).ignore());
                    }
                };
                effects.extend(
                    effect_builder
                        .put_signatures_to_storage(*known_signatures)
                        .ignore(),
                );
                effects
            }
            Event::IsBonded(None, _fs, true) => {
                // Unknown block but validator is bonded.
                // We should finalize the same block eventually. Either in this or in the
                // next era.
                Effects::new()
            }
            Event::IsBonded(_, fs, false) => {
                self.linear_chain_state.remove_from_pending_fs(&fs);
                // Unknown validator.
                let FinalitySignature {
                    public_key,
                    block_hash,
                    ..
                } = *fs;
                warn!(
                    validator = %public_key,
                    %block_hash,
                    "Received a signature from a validator that is not bonded."
                );
                // TODO: Disconnect from the sender.
                Effects::new()
            }
        }
    }
}
