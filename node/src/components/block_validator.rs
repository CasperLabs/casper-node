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
    collections::{hash_map::Entry, HashMap, HashSet},
    convert::Infallible,
    fmt::Debug,
    marker::PhantomData,
    sync::Arc,
};

use datasize::DataSize;
use derive_more::{Display, From};
use semver::Version;
use smallvec::{smallvec, SmallVec};
use tracing::error;

use crate::{
    components::Component,
    effect::{
        requests::{BlockValidationRequest, FetcherRequest, StorageRequest},
        EffectBuilder, EffectExt, EffectOptionExt, Effects, Responder,
    },
    types::{BlockLike, Deploy, DeployHash},
    Chainspec, NodeRng,
};
use keyed_counter::KeyedCounter;

use super::fetcher::FetchResult;

/// Block validator component event.
#[derive(Debug, From, Display)]
pub enum Event<T, I> {
    /// A request made of the block validator component.
    #[from]
    Request(BlockValidationRequest<T, I>),

    /// A deploy has been successfully found.
    #[display(fmt = "deploy {} found", _0)]
    DeployFound(DeployHash),

    /// A request to find a specific deploy, potentially from a peer, failed.
    #[display(fmt = "deploy {} missing", _0)]
    DeployMissing(DeployHash),

    /// An event changing the current state to BlockValidatorReady, once the chainspec has been
    /// loaded.
    #[display(fmt = "block validator loaded")]
    Loaded { chainspec: Arc<Chainspec> },
}

/// State of the current process of block validation.
///
/// Tracks whether or not there are deploys still missing and who is interested in the final result.
#[derive(DataSize, Debug)]
pub(crate) struct BlockValidationState<T> {
    /// The deploys that have not yet been "crossed off" the list of potential misses.
    missing_deploys: HashSet<DeployHash>,
    /// A list of responders that are awaiting an answer.
    responders: SmallVec<[Responder<(bool, T)>; 2]>,
}

#[derive(Debug)]
pub(crate) struct BlockValidatorReady<T, I> {
    /// Chainspec loaded for deploy validation.
    chainspec: Arc<Chainspec>,
    /// State of validation of a specific block.
    validation_states: HashMap<T, BlockValidationState<T>>,
    /// Number of requests for a specific deploy hash still in flight.
    in_flight: KeyedCounter<DeployHash>,

    _marker: PhantomData<I>,
}

impl<T, I> BlockValidatorReady<T, I>
where
    T: BlockLike + Send + Clone + 'static,
    I: Clone + Send + 'static,
{
    fn handle_event<REv>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        event: Event<T, I>,
    ) -> Effects<Event<T, I>>
    where
        REv: From<Event<T, I>>
            + From<BlockValidationRequest<T, I>>
            + From<StorageRequest>
            + From<FetcherRequest<I, Deploy>>
            + Send,
    {
        let mut effects = Effects::new();
        match event {
            Event::Request(BlockValidationRequest {
                block,
                sender,
                responder,
                block_timestamp,
            }) => {
                let block_deploys = block
                    .deploys()
                    .iter()
                    .map(|deploy_hash| **deploy_hash)
                    .collect::<HashSet<_>>();
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
                        }
                    }
                    Entry::Vacant(entry) => {
                        // Our entry is vacant - create an entry to track the state.
                        let missing_deploys: HashSet<DeployHash> =
                            entry.key().deploys().iter().map(|hash| **hash).collect();

                        let in_flight = &mut self.in_flight;
                        let chainspec = Arc::clone(&self.chainspec);
                        let fetch_effects: Effects<Event<T, I>> = block_deploys
                            .iter()
                            .flat_map(|deploy_hash| {
                                let chainspec = Arc::clone(&chainspec);
                                // For every request, increase the number of in-flight...
                                in_flight.inc(deploy_hash);

                                // ...then request it.
                                let deploy_hash = *deploy_hash;
                                let validate_deploy =
                                    move |result: FetchResult<Deploy, I>| match result {
                                        FetchResult::FromStorage(deploy)
                                        | FetchResult::FromPeer(deploy, _) => {
                                            if deploy.header().is_valid(
                                                &chainspec.genesis.deploy_config,
                                                block_timestamp,
                                            ) {
                                                Event::DeployFound(deploy_hash)
                                            } else {
                                                Event::DeployMissing(deploy_hash)
                                            }
                                        }
                                    };
                                effect_builder
                                    .fetch_deploy(deploy_hash, sender.clone())
                                    .map_or_else(validate_deploy, move || {
                                        Event::DeployMissing(deploy_hash)
                                    })
                            })
                            .collect();
                        effects.extend(fetch_effects);

                        entry.insert(BlockValidationState {
                            missing_deploys,
                            responders: smallvec![responder],
                        });
                    }
                }
            }
            Event::DeployFound(deploy_hash) => {
                // We successfully found a hash. Decrease the number of outstanding requests.
                self.in_flight.dec(&deploy_hash);

                // Our first pass updates all validation states, crossing off the found deploy.
                for state in self.validation_states.values_mut() {
                    state.missing_deploys.remove(&deploy_hash);
                }

                // Now we remove all states that have finished and notify the requestors.
                self.validation_states.retain(|key, state| {
                    if state.missing_deploys.is_empty() {
                        // This one is done and valid.
                        state.responders.drain(..).for_each(|responder| {
                            effects.extend(responder.respond((true, key.clone())).ignore());
                        });
                        false
                    } else {
                        true
                    }
                });
            }
            Event::DeployMissing(deploy_hash) => {
                // A deploy failed to fetch. If there is still hope (i.e. other outstanding
                // requests), we just ignore this little accident.
                if self.in_flight.dec(&deploy_hash) != 0 {
                    return Effects::new();
                }

                // Otherwise notify everyone still waiting on it that all is lost.
                self.validation_states.retain(|key, state| {
                    if state.missing_deploys.contains(&deploy_hash) {
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
            Event::Loaded { .. } => {}
        }
        effects
    }
}

/// Block validator states.
#[derive(Debug)]
pub(crate) enum BlockValidatorState<T, I> {
    Loading(Vec<Event<T, I>>),
    Ready(BlockValidatorReady<T, I>),
}

/// Block validator.
#[derive(DataSize, Debug)]
pub(crate) struct BlockValidator<T, I> {
    #[data_size(skip)]
    state: BlockValidatorState<T, I>,
}

impl<T, I> BlockValidator<T, I>
where
    T: BlockLike + Send + Clone + 'static,
    I: Clone + Send + 'static + Send,
{
    /// Creates a new block validator instance.
    pub(crate) fn new<REv>(effect_builder: EffectBuilder<REv>) -> (Self, Effects<Event<T, I>>)
    where
        REv: From<Event<T, I>> + From<StorageRequest> + Send + 'static,
    {
        let effects = async move {
            effect_builder
                .get_chainspec(Version::new(1, 0, 0))
                .await
                .expect("chainspec should be infallible")
        }
        .event(move |chainspec| Event::Loaded { chainspec });
        (
            BlockValidator {
                state: BlockValidatorState::Loading(Vec::new()),
            },
            effects,
        )
    }
}

impl<T, I, REv> Component<REv> for BlockValidator<T, I>
where
    T: BlockLike + Send + Clone + 'static,
    I: Clone + Send + 'static,
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
        match (&mut self.state, event) {
            (BlockValidatorState::Loading(requests), Event::Loaded { chainspec }) => {
                let mut new_ready_state = BlockValidatorReady {
                    chainspec,
                    validation_states: Default::default(),
                    in_flight: Default::default(),
                    _marker: PhantomData,
                };
                // Replay postponed events onto new state.
                for ev in requests.drain(..) {
                    effects.extend(new_ready_state.handle_event(effect_builder, ev));
                }
                self.state = BlockValidatorState::Ready(new_ready_state);
            }
            (
                BlockValidatorState::Loading(requests),
                request @ Event::Request(BlockValidationRequest { .. }),
            ) => {
                requests.push(request);
            }
            (BlockValidatorState::Loading(_), _deploy_found_or_missing) => {
                error!("Block validator reached unexpected state: should never receive a deploy from itself before it's ready.")
            }
            (BlockValidatorState::Ready(ref mut ready_state), event) => {
                effects.extend(ready_state.handle_event(effect_builder, event));
            }
        }
        effects
    }
}
