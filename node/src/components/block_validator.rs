//! Block validator
//!
//! The block validator checks whether all the deploys included in the proto block exist, either
//! locally or on the network.
//!
//! When multiple requests are made to validate the same proto block, they will eagerly return true
//! if valid, but only fail if all sources have been exhausted. This is only relevant when calling
//! for validation of the same protoblock multiple times at the same time.

mod keyed_counter;

use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap, HashSet, VecDeque},
    convert::Infallible,
    fmt::Debug,
    hash::Hash,
    sync::Arc,
};

use datasize::DataSize;
use derive_more::{Display, From};
use itertools::Itertools;
use smallvec::{smallvec, SmallVec};
use tracing::info;

use crate::{
    components::{block_proposer::DeployType, Component},
    effect::{
        requests::{BlockValidationRequest, FetcherRequest, StorageRequest},
        EffectBuilder, EffectExt, EffectOptionExt, Effects, Responder,
    },
    types::{appendable_block::AppendableBlock, Block, Chainspec, Deploy, DeployHash, ProtoBlock},
    NodeRng,
};
use keyed_counter::KeyedCounter;

use super::fetcher::FetchResult;

// TODO: Consider removing this trait.
pub trait BlockLike: Eq + Hash {
    fn deploys(&self) -> Vec<&DeployHash>;
}

impl BlockLike for Block {
    fn deploys(&self) -> Vec<&DeployHash> {
        self.deploy_hashes()
            .iter()
            .chain(self.transfer_hashes().iter())
            .collect()
    }
}

impl BlockLike for ProtoBlock {
    fn deploys(&self) -> Vec<&DeployHash> {
        self.deploys_and_transfers_iter().collect()
    }
}

/// Block validator component event.
#[derive(Debug, From, Display)]
pub enum Event<T, I> {
    /// A request made of the block validator component.
    #[from]
    Request(BlockValidationRequest<T, I>),

    /// A deploy has been successfully found.
    #[display(fmt = "deploy {} found", deploy_hash)]
    DeployFound {
        deploy_hash: DeployHash,
        deploy_type: Box<DeployType>,
    },

    /// A request to find a specific deploy, potentially from a peer, failed.
    #[display(fmt = "deploy {} missing", _0)]
    DeployMissing(DeployHash),

    /// Deploy was invalid. Unable to convert to a deploy type.
    #[display(fmt = "deploy {} invalid", _0)]
    CannotConvertDeploy(DeployHash),
}

/// State of the current process of block validation.
///
/// Tracks whether or not there are deploys still missing and who is interested in the final result.
#[derive(DataSize, Debug)]
pub(crate) struct BlockValidationState<T, I> {
    /// Appendable block ensuring that the deploys satisfy the validity conditions.
    appendable_block: AppendableBlock,
    /// The deploys that have not yet been "crossed off" the list of potential misses.
    missing_deploys: HashSet<DeployHash>,
    /// A list of responders that are awaiting an answer.
    responders: SmallVec<[Responder<(bool, T)>; 2]>,
    /// Peers that should have the data.
    sources: VecDeque<I>,
}

impl<T, I> BlockValidationState<T, I>
where
    I: PartialEq + Eq + 'static,
{
    /// Adds alternative source of data.
    /// Returns true if we already know about the peer.
    fn add_source(&mut self, peer: I) -> bool {
        if self.sources.contains(&peer) {
            true
        } else {
            self.sources.push_back(peer);
            false
        }
    }

    /// Returns a peer, if there is any, that we haven't yet tried.
    fn source(&mut self) -> Option<I> {
        self.sources.pop_front()
    }
}

#[derive(DataSize, Debug)]
pub(crate) struct BlockValidator<T, I> {
    /// Chainspec loaded for deploy validation.
    #[data_size(skip)]
    chainspec: Arc<Chainspec>,
    /// State of validation of a specific block.
    validation_states: HashMap<T, BlockValidationState<T, I>>,
    /// Number of requests for a specific deploy hash still in flight.
    in_flight: KeyedCounter<DeployHash>,
}

impl<T, I> BlockValidator<T, I>
where
    T: BlockLike + Debug + Send + Clone + 'static,
    I: Clone + Debug + Send + 'static + Send,
{
    /// Creates a new block validator instance.
    pub(crate) fn new(chainspec: Arc<Chainspec>) -> Self {
        BlockValidator {
            chainspec,
            validation_states: HashMap::new(),
            in_flight: KeyedCounter::default(),
        }
    }

    /// Prints a log message about an invalid block with duplicated deploys.
    fn log_block_with_replay(&self, sender: I, block: &T) {
        let mut deploy_counts = BTreeMap::new();
        for deploy_hash in block.deploys() {
            *deploy_counts.entry(*deploy_hash).or_default() += 1;
        }
        let duplicates = deploy_counts
            .into_iter()
            .filter_map(|(deploy_hash, count): (DeployHash, usize)| {
                (count > 1).then(|| format!("{} * {:?}", count, deploy_hash))
            })
            .join(", ");
        info!(
            ?sender, %duplicates,
            "received invalid block containing duplicated deploys"
        );
    }
}

impl<T, I, REv> Component<REv> for BlockValidator<T, I>
where
    T: BlockLike + Debug + Send + Clone + 'static,
    I: Clone + Debug + Send + PartialEq + Eq + 'static,
    REv: From<Event<T, I>>
        + From<BlockValidationRequest<T, I>>
        + From<FetcherRequest<I, Deploy>>
        + From<StorageRequest>
        + Send,
{
    type Event = Event<T, I>;
    type ConstructionError = Infallible;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        let mut effects = Effects::new();
        match event {
            Event::Request(BlockValidationRequest {
                block,
                sender,
                responder,
                block_timestamp,
            }) => {
                let block_deploys = block.deploys();
                let deploy_count = block_deploys.len();
                // Collect the deploys in a set; this also deduplicates them.
                let block_deploys: HashSet<_> = block_deploys
                    .iter()
                    .map(|deploy_hash| **deploy_hash)
                    .collect();
                if block_deploys.len() != deploy_count {
                    self.log_block_with_replay(sender, &block);
                    return responder.respond((false, block)).ignore();
                }
                if block_deploys.is_empty() {
                    // If there are no deploys, return early.
                    return responder.respond((true, block)).ignore();
                }

                // TODO: Clean this up to use `or_insert_with_key` once
                // https://github.com/rust-lang/rust/issues/71024 is stabilized.
                match self.validation_states.entry(block) {
                    Entry::Occupied(mut entry) => {
                        // The entry already exists.
                        if entry.get().missing_deploys.is_empty() {
                            // Block has already been validated successfully, early return to
                            // caller.
                            effects.extend(responder.respond((true, entry.key().clone())).ignore());
                        } else {
                            // We register ourselves as someone interested in the ultimate
                            // validation result.
                            entry.get_mut().responders.push(responder);
                            // And add an alternative source of data.
                            entry.get_mut().add_source(sender);
                        }
                    }
                    Entry::Vacant(entry) => {
                        // Our entry is vacant - create an entry to track the state.
                        let missing_deploys: HashSet<DeployHash> =
                            entry.key().deploys().iter().map(|hash| **hash).collect();

                        let in_flight = &mut self.in_flight;
                        let fetch_effects: Effects<Event<T, I>> = block_deploys
                            .iter()
                            .flat_map(|deploy_hash| {
                                // For every request, increase the number of in-flight...
                                in_flight.inc(deploy_hash);
                                // ...then request it.
                                fetch_deploy(effect_builder, *deploy_hash, sender.clone())
                            })
                            .collect();
                        effects.extend(fetch_effects);

                        let deploy_config = self.chainspec.deploy_config;
                        entry.insert(BlockValidationState {
                            appendable_block: AppendableBlock::new(deploy_config, block_timestamp),
                            missing_deploys,
                            responders: smallvec![responder],
                            sources: VecDeque::new(), /* This is empty b/c we create the first
                                                       * request using `sender`. */
                        });
                    }
                }
            }
            Event::DeployFound {
                deploy_hash,
                deploy_type,
            } => {
                // We successfully found a hash. Decrease the number of outstanding requests.
                self.in_flight.dec(&deploy_hash);

                // If a deploy is received for a given block that makes that block invalid somehow,
                // mark it for removal.
                let mut invalid = Vec::new();

                // Our first pass updates all validation states, crossing off the found deploy.
                for (key, state) in self.validation_states.iter_mut() {
                    if state.missing_deploys.remove(&deploy_hash) {
                        if let Err(err) = state.appendable_block.add(deploy_hash, &*deploy_type) {
                            // Notify everyone still waiting on it that all is lost.
                            info!(block=?key, %deploy_hash, ?deploy_type, ?err, "block invalid");
                            invalid.push(key.clone());
                        }
                    }
                }

                // Now we remove all states that have finished and notify the requestors.
                self.validation_states.retain(|key, state| {
                    if invalid.contains(key) {
                        state.responders.drain(..).for_each(|responder| {
                            effects.extend(responder.respond((false, key.clone())).ignore());
                        });
                        return false;
                    }
                    if state.missing_deploys.is_empty() {
                        // This one is done and valid.
                        state.responders.drain(..).for_each(|responder| {
                            effects.extend(responder.respond((true, key.clone())).ignore());
                        });
                        return false;
                    }
                    true
                });
            }
            Event::DeployMissing(deploy_hash) => {
                info!(%deploy_hash, "request to download deploy timed out");
                // A deploy failed to fetch. If there is still hope (i.e. other outstanding
                // requests), we just ignore this little accident.
                if self.in_flight.dec(&deploy_hash) != 0 {
                    return Effects::new();
                }

                // Flag indicating whether we've retried fetching the deploy.
                let mut retried = false;

                self.validation_states.retain(|key, state| {
                    if !state.missing_deploys.contains(&deploy_hash) {
                        return true
                    }
                    if retried {
                        // We don't want to retry downloading the same element more than once.
                        return true
                    }
                    match state.source() {
                        Some(peer) => {
                            info!(%deploy_hash, ?peer, "trying the next peer");
                            // There's still hope to download the deploy.
                            effects.extend(
                                fetch_deploy(effect_builder,
                                    deploy_hash,
                                    peer,
                                ));
                            retried = true;
                            true
                        },
                        None => {
                            // Notify everyone still waiting on it that all is lost.
                            info!(block=?key, %deploy_hash, "could not validate the deploy. block is invalid");
                            // This validation state contains a failed deploy hash, it can never
                            // succeed.
                            state.responders.drain(..).for_each(|responder| {
                                effects.extend(responder.respond((false, key.clone())).ignore());
                            });
                            false
                        }
                    }
                });

                if retried {
                    // If we retried, we need to increase this counter.
                    self.in_flight.inc(&deploy_hash);
                }
            }
            Event::CannotConvertDeploy(deploy_hash) => {
                info!(%deploy_hash, "cannot convert deploy to deploy type");
                // Deploy is invalid. There's no point waiting for other in-flight requests to
                // finish.
                self.in_flight.dec(&deploy_hash);

                self.validation_states.retain(|key, state| {
                    if state.missing_deploys.contains(&deploy_hash) {
                        // Notify everyone still waiting on it that all is lost.
                        info!(block=?key, %deploy_hash, "could not validate the deploy. block is invalid");
                        // This validation state contains a failed deploy hash, it can never
                        // succeed.
                        state.responders.drain(..).for_each(|responder| {
                            effects.extend(responder.respond((false, key.clone())).ignore());
                        });
                        false
                    } else {
                        true
                    }
                });
            }
        }
        effects
    }
}

/// Returns effects that fetch the deploy and validate it.
fn fetch_deploy<REv, T, I>(
    effect_builder: EffectBuilder<REv>,
    deploy_hash: DeployHash,
    sender: I,
) -> Effects<Event<T, I>>
where
    REv: From<Event<T, I>>
        + From<BlockValidationRequest<T, I>>
        + From<StorageRequest>
        + From<FetcherRequest<I, Deploy>>
        + Send,
    T: BlockLike + Debug + Send + Clone + 'static,
    I: Clone + Send + PartialEq + Eq + 'static,
{
    let validate_deploy = move |result: FetchResult<Deploy, I>| match result {
        FetchResult::FromStorage(deploy) | FetchResult::FromPeer(deploy, _) => deploy
            .deploy_type()
            .map_or(Event::CannotConvertDeploy(deploy_hash), |deploy_type| {
                Event::DeployFound {
                    deploy_hash,
                    deploy_type: Box::new(deploy_type),
                }
            }),
    };

    effect_builder
        .fetch_deploy(deploy_hash, sender)
        .map_or_else(validate_deploy, move || Event::DeployMissing(deploy_hash))
}
