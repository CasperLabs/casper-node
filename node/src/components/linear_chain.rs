use std::{
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    fmt::{self, Display, Formatter},
    marker::PhantomData,
};

use datasize::DataSize;
use derive_more::From;
use itertools::Itertools;
use tracing::{debug, error, info, warn};

use casper_types::{ExecutionResult, ProtocolVersion, PublicKey, SemVer, Signature};

use super::{consensus::EraId, Component};
use crate::{
    effect::{
        announcements::LinearChainAnnouncement,
        requests::{
            ConsensusRequest, ContractRuntimeRequest, LinearChainRequest, NetworkRequest,
            StorageRequest,
        },
        EffectBuilder, EffectExt, EffectOptionExt, EffectResultExt, Effects, Responder,
    },
    protocol::Message,
    types::{Block, BlockByHeight, BlockHash, BlockHeader, DeployHash, FinalitySignature},
    NodeRng,
};

use futures::FutureExt;

/// The maximum number of finality signatures from a single validator we keep in memory while
/// waiting for their block.
const MAX_PENDING_FINALITY_SIGNATURES_PER_VALIDATOR: usize = 1000;

impl<I> From<Box<FinalitySignature>> for Event<I> {
    fn from(fs: Box<FinalitySignature>) -> Self {
        Event::FinalitySignatureReceived(fs)
    }
}

#[derive(Debug, From)]
pub enum Event<I> {
    /// A linear chain request issued by another node in the network.
    #[from]
    Request(LinearChainRequest<I>),
    /// New linear chain block has been produced.
    LinearChainBlock {
        /// The block.
        block: Box<Block>,
        /// The deploys' execution results.
        execution_results: HashMap<DeployHash, ExecutionResult>,
    },
    /// A continuation for `GetBlock` scenario.
    GetBlockResult(BlockHash, Option<Box<Block>>, I),
    /// A continuation for `BlockAtHeight` scenario.
    GetBlockByHeightResult(u64, Option<Box<Block>>, I),
    /// A continuation for `BlockAtHeightLocal` scenario.
    GetBlockByHeightResultLocal(u64, Option<Box<Block>>, Responder<Option<Block>>),
    /// Finality signature received.
    /// Not necessarily _new_ finality signature.
    FinalitySignatureReceived(Box<FinalitySignature>),
    /// The result of putting a block to storage.
    PutBlockResult {
        /// The block.
        block: Box<Block>,
        /// The deploys' execution results.
        execution_results: HashMap<DeployHash, ExecutionResult>,
    },
    /// The result of requesting a block from storage to add a finality signature to it.
    GetBlockForFinalitySignaturesResult(Box<FinalitySignature>, Option<Box<Block>>),
    /// Check if validator is bonded in the future era.
    /// Validator's public key and the block's era are part of the finality signature.
    IsBondedFutureEra(Option<Box<Block>>, Box<FinalitySignature>),
    /// Result of testing if creator of the finality signature is bonded validator.
    IsBonded(Option<Box<Block>>, Box<FinalitySignature>, bool),
}

impl<I: Display> Display for Event<I> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Request(req) => write!(f, "linear chain request: {}", req),
            Event::LinearChainBlock { block, .. } => {
                write!(f, "linear chain new block: {}", block.hash())
            }
            Event::GetBlockResult(block_hash, maybe_block, peer) => write!(
                f,
                "linear chain get-block for {} from {} found: {}",
                block_hash,
                peer,
                maybe_block.is_some()
            ),
            Event::FinalitySignatureReceived(fs) => write!(
                f,
                "linear-chain new finality signature for block: {}, from: {}",
                fs.block_hash, fs.public_key,
            ),
            Event::PutBlockResult { .. } => write!(f, "linear-chain put-block result"),
            Event::GetBlockByHeightResult(height, result, peer) => write!(
                f,
                "linear chain get-block-height for height {} from {} found: {}",
                height,
                peer,
                result.is_some()
            ),
            Event::GetBlockByHeightResultLocal(height, block, _) => write!(
                f,
                "linear chain get-block-height-local for height={} found={}",
                height,
                block.is_some()
            ),
            Event::GetBlockForFinalitySignaturesResult(finality_signature, maybe_block) => {
                write!(
                    f,
                    "linear chain get-block-for-finality-signatures-result for {} found: {}",
                    finality_signature.block_hash,
                    maybe_block.is_some()
                )
            }
            Event::IsBonded(_block, fs, is_bonded) => {
                write!(
                    f,
                    "linear chain is-bonded for era {} validator {}, is_bonded: {}",
                    fs.era_id, fs.public_key, is_bonded
                )
            }
            Event::IsBondedFutureEra(_block, fs) => {
                write!(
                    f,
                    "linear chain is-bonded for future era {} validator {}",
                    fs.era_id, fs.public_key
                )
            }
        }
    }
}

// Cache for blocks.
// Blocks are invalidated when a block for newer era is inserted.
#[derive(DataSize, Debug)]
struct BlockCache {
    curr_era: EraId,
    blocks: HashMap<BlockHash, BlockHeader>,
    proofs: HashMap<BlockHash, BTreeMap<PublicKey, Signature>>,
}

impl BlockCache {
    fn new() -> Self {
        BlockCache {
            curr_era: EraId(0),
            blocks: Default::default(),
            proofs: Default::default(),
        }
    }

    fn get(&self, hash: &BlockHash) -> Option<Block> {
        let block_header = self.blocks.get(hash)?;
        let mut block = Block::from_header(block_header.clone());
        if let Some(proofs) = self.proofs.get(hash) {
            for (pub_key, sig) in proofs {
                block.append_proof(*pub_key, *sig);
            }
        }
        Some(block)
    }

    /// Inserts new block to the cache.
    /// Caches block's finality signatures as well.
    /// If we receive a block from a new era, old era blocks are removed.
    fn insert(&mut self, block: Block) {
        let new_block_era = block.header().era_id();
        if self.curr_era < new_block_era {
            // We optimistically assume that most of the signatures that arrive in close temporal
            // proximity refer to the same era.
            self.blocks.clear();
            self.proofs.clear();
            self.curr_era = new_block_era;
        }
        if !block.proofs().is_empty() {
            let entry = self.proofs.entry(*block.hash()).or_default();
            for (pk, sig) in block.proofs() {
                entry.insert(*pk, *sig);
            }
        }
        self.blocks.insert(*block.hash(), block.take_header());
    }
}

#[derive(DataSize, Debug)]
pub(crate) struct LinearChain<I> {
    /// The most recently added block.
    latest_block: Option<Block>,
    /// Finality signatures to be inserted in a block once it is available.
    pending_finality_signatures: HashMap<PublicKey, HashMap<BlockHash, FinalitySignature>>,
    block_cache: BlockCache,
    _marker: PhantomData<I>,
}

impl<I> LinearChain<I> {
    pub fn new() -> Self {
        LinearChain {
            latest_block: None,
            pending_finality_signatures: HashMap::new(),
            block_cache: BlockCache::new(),
            _marker: PhantomData,
        }
    }

    // TODO: Remove once we can return all linear chain blocks from persistent storage.
    pub fn latest_block(&self) -> &Option<Block> {
        &self.latest_block
    }

    // Checks if we have already enqueued that finality signature.
    fn has_finality_signature(&self, fs: &FinalitySignature) -> bool {
        let creator = fs.public_key;
        let block_hash = fs.block_hash;
        self.pending_finality_signatures
            .get(&creator)
            .map_or(false, |sigs| sigs.contains_key(&block_hash))
    }

    /// Removes all entries for which there are no finality signatures.
    fn remove_empty_entries(&mut self) {
        self.pending_finality_signatures
            .retain(|_, sigs| !sigs.is_empty());
    }

    /// Adds pending finality signatures to the block; returns events to announce and broadcast
    /// them, and the updated block.
    fn collect_pending_finality_signatures<REv>(
        &mut self,
        mut block: Block,
        effect_builder: EffectBuilder<REv>,
    ) -> (Block, Effects<Event<I>>)
    where
        REv: From<StorageRequest>
            + From<ConsensusRequest>
            + From<NetworkRequest<I, Message>>
            + From<LinearChainAnnouncement>
            + Send,
        I: Display + Send + 'static,
    {
        let mut effects = Effects::new();
        let block_hash = block.hash();
        let pending_sigs = self
            .pending_finality_signatures
            .values_mut()
            .filter_map(|sigs| sigs.remove(&block_hash).map(Box::new))
            .filter(|fs| !block.proofs().contains_key(&fs.public_key))
            .collect_vec();
        self.remove_empty_entries();
        let block_era = block.header().era_id();
        // Add new signatures and send the updated block to storage.
        for fs in pending_sigs {
            if fs.era_id != block_era {
                // finality signature was created with era id that doesn't match block's era.
                // TODO: disconnect from the sender.
                continue;
            }
            if block.proofs().contains_key(&fs.public_key) {
                // Don't send finality signatures we already know of.
                continue;
            }
            block.append_proof(fs.public_key, fs.signature);
            let message = Message::FinalitySignature(fs.clone());
            effects.extend(effect_builder.broadcast_message(message).ignore());
            effects.extend(effect_builder.announce_finality_signature(fs).ignore());
        }
        (block, effects)
    }

    /// Adds finality signature to the collection of pending finality signatures.
    fn add_pending_finality_signature(&mut self, fs: FinalitySignature) {
        let FinalitySignature {
            block_hash,
            public_key,
            ..
        } = fs;
        debug!(%block_hash, %public_key, "received new finality signature");
        let sigs = self
            .pending_finality_signatures
            .entry(public_key)
            .or_default();
        // Limit the memory we use for storing unknown signatures from each validator.
        if sigs.len() >= MAX_PENDING_FINALITY_SIGNATURES_PER_VALIDATOR {
            warn!(
                %block_hash, %public_key,
                "received too many finality signatures for unknown blocks"
            );
            return;
        }
        // Add the pending signature.
        let _ = sigs.insert(block_hash, fs);
    }

    /// Removes finality signature from the pending collection.
    fn remove_from_pending_fs(&mut self, fs: &FinalitySignature) {
        let FinalitySignature {
            block_hash,
            era_id: _era_id,
            signature: _signature,
            public_key,
        } = fs;
        debug!(%block_hash, %public_key, "removing finality signature from pending collection");
        if let Some(validator_sigs) = self.pending_finality_signatures.get_mut(public_key) {
            validator_sigs.remove(&block_hash);
        }
        self.remove_empty_entries();
    }
}

impl<I, REv> Component<REv> for LinearChain<I>
where
    REv: From<StorageRequest>
        + From<ConsensusRequest>
        + From<NetworkRequest<I, Message>>
        + From<LinearChainAnnouncement>
        + From<ContractRuntimeRequest>
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
            Event::Request(LinearChainRequest::BlockRequest(block_hash, sender)) => effect_builder
                .get_block_from_storage(block_hash)
                .event(move |maybe_block| {
                    Event::GetBlockResult(block_hash, maybe_block.map(Box::new), sender)
                }),
            Event::Request(LinearChainRequest::BlockAtHeightLocal(height, responder)) => {
                effect_builder
                    .get_block_at_height_from_storage(height)
                    .event(move |block| {
                        Event::GetBlockByHeightResultLocal(height, block.map(Box::new), responder)
                    })
            }
            Event::Request(LinearChainRequest::BlockAtHeight(height, sender)) => effect_builder
                .get_block_at_height_from_storage(height)
                .event(move |maybe_block| {
                    Event::GetBlockByHeightResult(height, maybe_block.map(Box::new), sender)
                }),
            Event::GetBlockByHeightResultLocal(_height, block, responder) => {
                responder.respond(block.map(|boxed| *boxed)).ignore()
            }
            Event::GetBlockByHeightResult(block_height, maybe_block, sender) => {
                let block_at_height = match maybe_block {
                    None => {
                        debug!("failed to get {} for {}", block_height, sender);
                        BlockByHeight::Absent(block_height)
                    }
                    Some(block) => BlockByHeight::new(*block),
                };
                match Message::new_get_response(&block_at_height) {
                    Ok(message) => effect_builder.send_message(sender, message).ignore(),
                    Err(error) => {
                        error!("failed to create get-response {}", error);
                        Effects::new()
                    }
                }
            }
            Event::GetBlockResult(block_hash, maybe_block, sender) => match maybe_block {
                None => {
                    debug!("failed to get {} for {}", block_hash, sender);
                    Effects::new()
                }
                Some(block) => match Message::new_get_response(&*block) {
                    Ok(message) => effect_builder.send_message(sender, message).ignore(),
                    Err(error) => {
                        error!("failed to create get-response {}", error);
                        Effects::new()
                    }
                },
            },
            Event::LinearChainBlock {
                block,
                execution_results,
            } => {
                let (block, mut effects) =
                    self.collect_pending_finality_signatures(*block, effect_builder);
                // Cache the block as we expect more finality signatures to arrive soon.
                self.block_cache.insert(block.clone());
                let block = Box::new(block);
                effects.extend(effect_builder.put_block_to_storage(block.clone()).event(
                    move |_| Event::PutBlockResult {
                        block,
                        execution_results,
                    },
                ));
                effects
            }
            Event::PutBlockResult {
                block,
                execution_results,
            } => {
                self.latest_block = Some(*block.clone());

                let block_header = block.take_header();
                let block_hash = block_header.hash();
                let era_id = block_header.era_id();
                let height = block_header.height();
                info!(%block_hash, %era_id, %height, "linear chain block stored");
                let mut effects = effect_builder
                    .put_execution_results_to_storage(block_hash, execution_results)
                    .ignore();
                effects.extend(
                    effect_builder
                        .handle_linear_chain_block(block_header.clone())
                        .map_some(move |fs| Event::FinalitySignatureReceived(Box::new(fs))),
                );
                effects.extend(
                    effect_builder
                        .announce_block_added(block_hash, block_header)
                        .ignore(),
                );
                effects
            }
            Event::FinalitySignatureReceived(fs) => {
                let FinalitySignature {
                    block_hash,
                    public_key,
                    ..
                } = *fs;
                if let Err(err) = fs.verify() {
                    warn!(%block_hash, %public_key, %err, "received invalid finality signature");
                    return Effects::new();
                }
                if self.has_finality_signature(&fs) {
                    debug!(block_hash=%fs.block_hash, public_key=%fs.public_key,
                        "finality signature already pending");
                    return Effects::new();
                }
                self.add_pending_finality_signature(*fs.clone());
                match self.block_cache.get(&block_hash) {
                    None => effect_builder.get_block_from_storage(block_hash).event(
                        move |maybe_block| {
                            let maybe_box_block = maybe_block.map(Box::new);
                            Event::GetBlockForFinalitySignaturesResult(fs, maybe_box_block)
                        },
                    ),
                    Some(block) => effect_builder.immediately().event(move |_| {
                        Event::GetBlockForFinalitySignaturesResult(fs, Some(Box::new(block)))
                    }),
                }
            }
            Event::GetBlockForFinalitySignaturesResult(fs, maybe_block) => {
                if let Some(block) = &maybe_block {
                    if block.header().era_id() != fs.era_id {
                        warn!(public_key=%fs.public_key, "Finality signature with invalid era id.");
                        // TODO: Disconnect from the sender.
                        return Effects::new();
                    }
                    // Populate cache so that next finality signatures don't have to read from the
                    // storage. If block is already from cache then this will be a noop.
                    self.block_cache.insert(*block.clone())
                }
                // Check if validator is bonded in the era in which the block was created.
                effect_builder
                    .is_bonded_validator(fs.era_id, fs.public_key)
                    .map(|is_bonded| {
                        if is_bonded {
                            Ok((maybe_block, fs, is_bonded))
                        } else {
                            // If it's not bonded in that era, we will check if it's bonded in the
                            // future era.
                            Err((maybe_block, fs))
                        }
                    })
            }
            .result(
                |(maybe_block, fs, is_bonded)| Event::IsBonded(maybe_block, fs, is_bonded),
                |(maybe_block, fs)| Event::IsBondedFutureEra(maybe_block, fs),
            ),
            Event::IsBondedFutureEra(maybe_block, fs) => {
                match self.latest_block.as_ref() {
                    // If we don't have any block yet, we cannot determine who is bonded or not.
                    None => effect_builder
                        .immediately()
                        .event(move |_| Event::IsBonded(maybe_block, fs, false)),
                    Some(block) => {
                        let latest_header = block.header();
                        let state_root_hash = latest_header.state_root_hash();
                        // TODO: Use protocol version that is valid for the block's height.
                        let protocol_version = ProtocolVersion::new(SemVer::V1_0_0);
                        effect_builder
                            .is_bonded_in_future_era(
                                *state_root_hash,
                                fs.era_id,
                                protocol_version,
                                fs.public_key,
                            )
                            .map(|res| {
                                match res {
                                    // Promote this error to a non-error case.
                                    // It's not an error that we can't find the era that this
                                    // FinalitySignature is for.
                                    Err(error) if error.is_era_validators_missing() => Ok(false),
                                    _ => res,
                                }
                            })
                            .result(
                                |is_bonded| Event::IsBonded(maybe_block, fs, is_bonded),
                                |error| {
                                    error!(%error, "is_bonded_in_future_era returned an error.");
                                    panic!("couldn't check if validator is bonded")
                                },
                            )
                    }
                }
            }
            Event::IsBonded(Some(mut block), fs, true) => {
                // Known block and signature from a bonded validator.
                // Check if we had already seen this signature before.
                let signature_known = block
                    .proofs()
                    .get(&fs.public_key)
                    .iter()
                    .any(|sig| *sig == &fs.signature);
                // If new, gossip and store.
                if signature_known {
                    Effects::new()
                } else {
                    let message = Message::FinalitySignature(fs.clone());
                    let mut effects = effect_builder.broadcast_message(message).ignore();
                    effects.extend(
                        effect_builder
                            .announce_finality_signature(fs.clone())
                            .ignore(),
                    );
                    block.append_proof(fs.public_key, fs.signature);
                    // Cache the results in case we receive the same finality signature before we
                    // manage to store it in the database.
                    self.block_cache.insert(*block.clone());
                    effects.extend(effect_builder.put_block_to_storage(block).ignore());
                    effects
                }
            }
            Event::IsBonded(None, _, true) => {
                // Unknown block but validator is bonded.
                // We should finalize the same block eventually. Either in this or in the
                // next era.
                Effects::new()
            }
            Event::IsBonded(Some(_), fs, false) | Event::IsBonded(None, fs, false) => {
                self.remove_from_pending_fs(&fs);
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
