//! Linear chain synchronizer.
//!
//! Synchronizes the linear chain when node joins the network.
//!
//! Steps are:
//! 1. Fetch blocks up to initial, trusted hash (blocks are downloaded starting from trusted hash up
//! until Genesis).
//! 2. Fetch deploys of the lowest height block.
//! 3. Execute that block.
//! 4. Repeat steps 2-3 until trusted hash is reached.
//! 5. Transition to `SyncingDescendants` state.
//! 6. Fetch child block of highest block.
//! 7. Fetch deploys of that block.
//! 8. Execute that block.
//! 9. Repeat steps 6-8 as long as there's a child in the linear chain.
//!
//! The order of "download block – download deploys – execute" block steps differ,
//! in order to increase the chances of catching up with the linear chain quicker.
//! When synchronizing linear chain up to the trusted hash we cannot execute later blocks without
//! earlier ones. When we're syncing descendants, on the other hand, we can and we want to do it
//! ASAP so that we can start participating in consensus. That's why deploy fetching and block
//! execution is interleaved. If we had downloaded the whole chain, and then deploys, and then
//! execute (as we do in the first, SynchronizeTrustedHash, phase) it would have taken more time and
//! we might miss more eras.

mod error;
mod event;
mod metrics;
mod operations;
mod peers;
mod state;
mod traits;

use std::{
    convert::Infallible,
    fmt::{Debug, Display},
    mem,
    str::FromStr,
};

use datasize::DataSize;
use prometheus::Registry;
use tracing::{error, info, trace, warn};

use casper_types::{EraId, Key, ProtocolVersion};

use self::event::{BlockByHashResult, DeploysResult};

use super::{
    storage::{self, Storage},
    Component,
};
use crate::{
    components::fetcher::{FetchedData, FetcherError},
    effect::{
        requests::{FetcherRequest, NetworkInfoRequest},
        EffectBuilder, EffectExt, EffectOptionExt, Effects,
    },
    fatal,
    types::{
        ActivationPoint, Block, BlockHash, BlockHeader, BlockHeaderWithMetadata, BlockWithMetadata,
        Chainspec, Deploy, FinalizedBlock, TimeDiff, Timestamp,
    },
    NodeRng,
};
use casper_execution_engine::{shared::stored_value::StoredValue, storage::trie::Trie};
use event::BlockByHeightResult;
pub use event::Event;
pub use metrics::LinearChainSyncMetrics;
pub use peers::PeersState;
pub use state::State;
pub use traits::ReactorEventT;
// TODO: Move this out of linear_chain_sync.
pub(crate) use operations::weigh_finality_signatures;

#[derive(DataSize, Debug)]
pub(crate) struct LinearChainSync<I> {
    init_hash: Option<BlockHash>,
    chainspec: Chainspec,
    started: bool,

    peers: PeersState<I>,
    state: State,
    #[data_size(skip)]
    metrics: LinearChainSyncMetrics,
    /// The next upgrade activation point.
    /// When we download the switch block of an era immediately before the activation point,
    /// we need to shut down for an upgrade.
    next_upgrade_activation_point: Option<ActivationPoint>,
    stop_for_upgrade: bool,
    /// Key for storing the linear chain sync state.
    state_key: Vec<u8>,
    /// Shortest era that is allowed with the given protocol configuration.
    shortest_era: TimeDiff,
    /// Minimum round length that is allowed with the given protocol configuration.
    min_round_length: TimeDiff,
    /// Flag indicating whether we managed to sync at least one block.
    started_syncing: bool,
    /// The protocol version the node is currently running with.
    protocol_version: ProtocolVersion,
}

impl<I: Clone + PartialEq + 'static> LinearChainSync<I> {
    // TODO: fix this
    #[allow(clippy::too_many_arguments)]
    pub fn new<REv, Err>(
        registry: &Registry,
        effect_builder: EffectBuilder<REv>,
        chainspec: &Chainspec,
        storage: &Storage,
        init_hash: Option<BlockHash>,
        highest_block: Option<Block>,
        next_upgrade_activation_point: Option<ActivationPoint>,
    ) -> Result<(Self, Effects<Event<I>>), Err>
    where
        REv: From<Event<I>> + Send,
        Err: From<prometheus::Error> + From<storage::Error>,
    {
        // set timeout to 5 minutes after now.
        let five_minutes = TimeDiff::from_str("5minutes").unwrap();
        let timeout_event = effect_builder
            .set_timeout(five_minutes.into())
            .event(|_| Event::InitializeTimeout);
        let protocol_version = chainspec.protocol_config.version;
        if let Some(state) = read_init_state(storage, chainspec)? {
            let linear_chain_sync = LinearChainSync::from_state(
                init_hash,
                registry,
                chainspec,
                state,
                next_upgrade_activation_point,
                protocol_version,
            )?;
            Ok((linear_chain_sync, timeout_event))
        } else {
            // Shortest era is the maximum of the two.
            let shortest_era: TimeDiff = std::cmp::max(
                chainspec.highway_config.min_round_length()
                    * chainspec.core_config.minimum_era_height,
                chainspec.core_config.era_duration,
            );
            let state = match init_hash {
                Some(init_hash) => State::sync_trusted_hash(
                    init_hash,
                    highest_block.map(|block| block.take_header()),
                ),
                None => State::Done(highest_block.map(Box::new)),
            };
            let state_key = create_state_key(&chainspec);
            let linear_chain_sync = LinearChainSync {
                init_hash,
                chainspec: chainspec.clone(),
                started: false,
                peers: PeersState::new(),
                state,
                metrics: LinearChainSyncMetrics::new(registry)?,
                next_upgrade_activation_point,
                stop_for_upgrade: false,
                state_key,
                shortest_era,
                min_round_length: chainspec.highway_config.min_round_length(),
                started_syncing: false,
                protocol_version,
            };
            Ok((linear_chain_sync, timeout_event))
        }
    }

    /// Initialize `LinearChainSync` component from preloaded `State`.
    fn from_state(
        init_hash: Option<BlockHash>,
        registry: &Registry,
        chainspec: &Chainspec,
        state: State,
        next_upgrade_activation_point: Option<ActivationPoint>,
        protocol_version: ProtocolVersion,
    ) -> Result<Self, prometheus::Error> {
        let state_key = create_state_key(chainspec);
        info!(?state, "reusing previous state");
        // Shortest era is the maximum of the two.
        let shortest_era: TimeDiff = std::cmp::max(
            chainspec.highway_config.min_round_length() * chainspec.core_config.minimum_era_height,
            chainspec.core_config.era_duration,
        );
        Ok(LinearChainSync {
            init_hash,
            chainspec: chainspec.clone(),
            started: false,
            peers: PeersState::new(),
            state,
            metrics: LinearChainSyncMetrics::new(registry)?,
            next_upgrade_activation_point,
            stop_for_upgrade: false,
            state_key,
            shortest_era,
            min_round_length: chainspec.highway_config.min_round_length(),
            started_syncing: false,
            protocol_version,
        })
    }

    /// Add new block to linear chain.
    fn add_block(&mut self, block: Block) {
        self.started_syncing = true;
        match &mut self.state {
            State::None | State::Done(_) => {}
            State::SyncingTrustedHash { linear_chain, .. } => linear_chain.push(block),
            State::SyncingDescendants { latest_block, .. } => **latest_block = block,
        };
    }

    /// Returns `true` if we have finished syncing linear chain.
    pub fn is_synced(&self) -> bool {
        matches!(self.state, State::Done(_))
    }

    /// Returns `true` if we should stop for upgrade.
    pub fn stopped_for_upgrade(&self) -> bool {
        self.stop_for_upgrade
    }

    fn block_downloaded<REv>(
        &mut self,
        rng: &mut NodeRng,
        effect_builder: EffectBuilder<REv>,
        block: &Block,
    ) -> Effects<Event<I>>
    where
        I: Debug + Send + Eq + 'static,
        REv: ReactorEventT<I>,
    {
        self.peers.reset(rng);
        self.state.block_downloaded(block);
        self.add_block(block.clone());
        match &self.state {
            State::None | State::Done(_) => {
                error!(state=?self.state, "block downloaded when in incorrect state.");
                fatal!(effect_builder, "block downloaded in incorrect state").ignore()
            }
            State::SyncingTrustedHash {
                highest_block_header,
                ..
            } => {
                let should_start_downloading_deploys = highest_block_header
                    .as_ref()
                    .map(|hdr| hdr.hash() == *block.header().parent_hash())
                    .unwrap_or(false)
                    || block.header().is_genesis_child();
                if should_start_downloading_deploys {
                    info!("linear chain downloaded. Start downloading deploys.");
                    effect_builder
                        .immediately()
                        .event(move |_| Event::StartDownloadingDeploys)
                } else {
                    self.fetch_next_block(effect_builder, rng, block)
                }
            }
            State::SyncingDescendants { .. } => {
                // When synchronizing descendants, we want to download block and execute it
                // before trying to download the next block in linear chain.
                self.fetch_next_block_deploys(effect_builder)
            }
        }
    }

    fn mark_done(&mut self, latest_block: Option<Block>) {
        let latest_block = latest_block.map(Box::new);
        self.state = State::Done(latest_block);
    }

    /// Handles an event indicating that a linear chain block has been executed and handled by
    /// consensus component. This is a signal that we can safely continue with the next blocks,
    /// without worrying about timing and/or ordering issues.
    /// Returns effects that are created as a response to that event.
    fn block_handled<REv>(
        &mut self,
        rng: &mut NodeRng,
        effect_builder: EffectBuilder<REv>,
        block: Block,
    ) -> Effects<Event<I>>
    where
        I: Debug + Eq + Send + 'static,
        REv: ReactorEventT<I>,
    {
        let height = block.height();
        let hash = block.hash();
        trace!(%hash, %height, "downloaded linear chain block.");
        if block.header().is_switch_block() {
            self.state
                .set_last_switch_block_header(block.header().clone());
        }
        if block.header().is_switch_block() && self.should_upgrade(block.header().era_id()) {
            info!(
                era = block.header().era_id().value(),
                "shutting down for upgrade"
            );
            return effect_builder
                .immediately()
                .event(|_| Event::InitUpgradeShutdown);
        }
        // Reset peers before creating new requests.
        self.peers.reset(rng);
        let block_height = block.height();
        let curr_state = mem::replace(&mut self.state, State::None);
        match curr_state {
            State::None | State::Done(_) => {
                error!(state=?self.state, "block handled when in incorrect state.");
                fatal!(effect_builder, "block handled in incorrect state").ignore()
            }
            // Keep syncing from genesis if we haven't reached the trusted block hash
            State::SyncingTrustedHash {
                highest_block_seen,
                ref latest_block,
                ..
            } if highest_block_seen != block_height => {
                match latest_block.as_ref() {
                    Some(expected) if expected != &block => {
                        error!(
                            ?expected, got=?block,
                            "block execution result doesn't match received block"
                        );
                        return fatal!(effect_builder, "unexpected block execution result")
                            .ignore();
                    }
                    None => {
                        error!("block execution results received when not expected");
                        return fatal!(effect_builder, "unexpected block execution results.")
                            .ignore();
                    }
                    Some(_) => (),
                }
                self.state = curr_state;
                self.fetch_next_block_deploys(effect_builder)
            }
            // Otherwise transition to State::SyncingDescendants
            State::SyncingTrustedHash {
                highest_block_seen,
                trusted_hash,
                ref latest_block,
                last_switch_block_header,
                ..
            } => {
                assert_eq!(highest_block_seen, block_height);
                match latest_block.as_ref() {
                    Some(expected) if expected != &block => {
                        error!(
                            ?expected, got=?block,
                            "block execution result doesn't match received block"
                        );
                        return fatal!(effect_builder, "unexpected block execution result")
                            .ignore();
                    }
                    None => {
                        error!("block execution results received when not expected");
                        return fatal!(effect_builder, "unexpected block execution results.")
                            .ignore();
                    }
                    Some(_) => (),
                }
                info!(%block_height, "Finished synchronizing linear chain up until trusted hash.");
                let peer = self.peers.random_unsafe();
                // Kick off syncing trusted hash descendants.
                self.state = State::sync_descendants(trusted_hash, block, last_switch_block_header);
                fetch_block_at_height(effect_builder, peer, block_height + 1)
            }
            State::SyncingDescendants {
                ref latest_block,
                ref last_switch_block_header,
                ..
            } => {
                if latest_block.as_ref() != &block {
                    error!(
                        expected=?*latest_block, got=?block,
                        "block execution result doesn't match received block"
                    );
                    return fatal!(effect_builder, "unexpected block execution result").ignore();
                }
                match operations::is_current_era::<I>(
                    latest_block.header(),
                    last_switch_block_header.as_ref().map(Box::as_ref),
                    &self.chainspec,
                    Timestamp::now(),
                ) {
                    Err(err) => {
                        fatal!(effect_builder, "failed to compute era duration: {:?}", err).ignore()
                    }
                    Ok(true) => {
                        info!(
                            hash=?block.hash(),
                            height=?block.header().height(),
                            era=block.header().era_id().value(),
                            "downloaded a block in the current era. finished synchronization"
                        );
                        self.mark_done(Some(*latest_block.clone()));
                        Effects::new()
                    }
                    Ok(false) => {
                        self.state = curr_state;
                        self.fetch_next_block(effect_builder, rng, &block)
                    }
                }
            }
        }
    }

    /// Returns effects for fetching next block's deploys.
    fn fetch_next_block_deploys<REv>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
    ) -> Effects<Event<I>>
    where
        I: Debug + Eq + Send + 'static,
        REv: ReactorEventT<I>,
    {
        let peer = self.peers.random_unsafe();

        let next_block = match &mut self.state {
            State::None | State::Done(_) => {
                error!(state=?self.state, "tried fetching next block when in wrong state");
                return fatal!(
                    effect_builder,
                    "tried fetching next block when in wrong state"
                )
                .ignore();
            }
            State::SyncingTrustedHash {
                linear_chain,
                latest_block,
                ..
            } => match linear_chain.pop() {
                None => None,
                Some(block) => {
                    // Update `latest_block` so that we can verify whether result of execution
                    // matches the expected value.
                    latest_block.replace(block.clone());
                    Some(block)
                }
            },
            State::SyncingDescendants { latest_block, .. } => Some((**latest_block).clone()),
        };

        next_block.map_or_else(
            || {
                warn!("tried fetching next block deploys when there was no block.");
                Effects::new()
            },
            |block| {
                self.metrics.reset_start_time();
                fetch_block_deploys(effect_builder, peer, block)
            },
        )
    }

    fn fetch_next_block<REv>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        rng: &mut NodeRng,
        block: &Block,
    ) -> Effects<Event<I>>
    where
        I: Debug + Eq + Send + 'static,
        REv: ReactorEventT<I>,
    {
        self.peers.reset(rng);
        let peer = self.peers.random_unsafe();
        match self.state {
            State::SyncingTrustedHash { .. } => {
                let parent_hash = *block.header().parent_hash();
                self.metrics.reset_start_time();
                fetch_block_by_hash(effect_builder, peer, parent_hash)
            }
            State::SyncingDescendants { .. } => {
                let next_height = block.height() + 1;
                self.metrics.reset_start_time();
                fetch_block_at_height(effect_builder, peer, next_height)
            }
            State::Done(_) | State::None => {
                error!(state=?self.state, "tried fetching next block when in wrong state");
                fatal!(
                    effect_builder,
                    "tried fetching next block when in wrong state"
                )
                .ignore()
            }
        }
    }

    fn handle_upgrade_shutdown<REv>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
    ) -> Effects<Event<I>>
    where
        I: Debug + Eq + Send + 'static,
        REv: ReactorEventT<I>,
    {
        if self.state.is_done() || self.state.is_none() {
            error!(state=?self.state, "shutdown for upgrade initiated when in wrong state");
            return fatal!(
                effect_builder,
                "shutdown for upgrade initiated when in wrong state"
            )
            .ignore();
        }
        effect_builder
            .save_state(self.state_key.clone().into(), Some(self.state.clone()))
            .event(|_| Event::Shutdown(true))
    }

    pub(crate) fn latest_block(&self) -> Option<&Block> {
        match &self.state {
            State::SyncingTrustedHash { latest_block, .. } => Option::as_ref(&*latest_block),
            State::SyncingDescendants { latest_block, .. } => Some(&*latest_block),
            State::Done(latest_block) => latest_block.as_deref(),
            State::None => None,
        }
    }

    pub fn into_maybe_latest_block_header(self) -> Option<BlockHeader> {
        match self.state {
            State::SyncingTrustedHash { latest_block, .. } => latest_block.map(Block::take_header),
            State::SyncingDescendants { latest_block, .. } => Some(latest_block.take_header()),
            State::Done(maybe_latest_block) => {
                maybe_latest_block.map(|latest_block| latest_block.take_header())
            }
            State::None => None,
        }
    }

    fn should_upgrade(&self, era_id: EraId) -> bool {
        match self.next_upgrade_activation_point {
            None => false,
            Some(activation_point) => activation_point.should_upgrade(&era_id),
        }
    }

    fn set_last_block_if_syncing_trusted_hash(&mut self, block: &Block) {
        if let State::SyncingTrustedHash {
            ref mut latest_block,
            ..
        } = &mut self.state
        {
            *latest_block = Box::new(Some(block.clone()));
        }
        self.state.block_downloaded(block);
    }
}

impl<I, REv> Component<REv> for LinearChainSync<I>
where
    I: Debug + Display + Clone + Eq + Send + PartialEq + Sync + 'static,
    REv: ReactorEventT<I>
        + From<FetcherRequest<I, Deploy>>
        + From<FetcherRequest<I, BlockHeader>>
        + From<FetcherRequest<I, Trie<Key, StoredValue>>>
        + From<FetcherRequest<I, BlockHeaderWithMetadata>>
        + From<NetworkInfoRequest<I>>,
{
    type Event = Event<I>;
    type ConstructionError = Infallible;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Start(init_peer) => {
                match &self.state {
                    State::None => {
                        // No syncing configured.
                        trace!("received `Start` event when in {} state.", self.state);
                        Effects::new()
                    }
                    State::Done(_) => {
                        // Illegal states for syncing start.
                        error!("should not have received `Start` event when in `Done` state.",);
                        Effects::new()
                    }
                    State::SyncingDescendants { latest_block, .. } => {
                        let next_block_height = latest_block.height() + 1;
                        info!(?next_block_height, "start synchronization");
                        self.metrics.reset_start_time();
                        fetch_block_at_height(effect_builder, init_peer, next_block_height)
                    }
                    State::SyncingTrustedHash { trusted_hash, .. } => {
                        trace!(?trusted_hash, "start synchronization");
                        // Start synchronization.
                        self.metrics.reset_start_time();
                        fetch_block_by_hash(effect_builder, init_peer, *trusted_hash)
                    }
                }
            }
            Event::GetBlockHeightResult(block_height, fetch_result) => {
                match fetch_result {
                    BlockByHeightResult::Absent(peer) => {
                        self.metrics.observe_get_block_by_height();
                        trace!(
                            %block_height, %peer,
                            "failed to download block by height. Trying next peer"
                        );
                        self.peers.failure(&peer);
                        match self.peers.random() {
                            None => {
                                // `block_height` not found on any of the peers.
                                // We have synchronized all, currently existing, descendants of
                                // trusted hash.
                                info!(
                                    "finished synchronizing descendants of the trusted hash. \
                                    cleaning state."
                                );
                                self.mark_done(self.latest_block().cloned());
                                Effects::new()
                            }
                            Some(peer) => {
                                self.metrics.reset_start_time();
                                fetch_block_at_height(effect_builder, peer, block_height)
                            }
                        }
                    }
                    BlockByHeightResult::FromStorage(block) => {
                        // We shouldn't get invalid data from the storage.
                        // If we do, it's a bug.
                        assert_eq!(block.height(), block_height, "Block height mismatch.");
                        assert_eq!(
                            block.protocol_version(),
                            self.protocol_version,
                            "block protocol version mismatch"
                        );
                        trace!(%block_height, "Linear block found in the local storage.");
                        // When syncing descendants of a trusted hash, we might have some of
                        // them in our local storage. If that's the case, just continue.
                        self.block_downloaded(rng, effect_builder, &block)
                    }
                    BlockByHeightResult::FromPeer(block, peer) => {
                        self.metrics.observe_get_block_by_height();
                        trace!(%block_height, %peer, "linear chain block downloaded from a peer");
                        if block.height() != block_height
                            || *block.header().parent_hash() != *self.latest_block().unwrap().hash()
                        {
                            warn!(
                                %peer,
                                got_height = block.height(),
                                expected_height = block_height,
                                got_parent = %block.header().parent_hash(),
                                expected_parent = %self.latest_block().unwrap().hash(),
                                "block mismatch",
                            );
                            // NOTE: Signal misbehaving validator to networking layer.
                            self.peers.ban(&peer);
                            return self.handle_event(
                                effect_builder,
                                rng,
                                Event::GetBlockHeightResult(
                                    block_height,
                                    BlockByHeightResult::Absent(peer),
                                ),
                            );
                        }
                        if block.protocol_version() != self.protocol_version {
                            warn!(
                                %peer,
                                protocol_version = %self.protocol_version,
                                block_version = %block.protocol_version(),
                                "block protocol version mismatch",
                            );
                            // NOTE: Signal misbehaving validator to networking layer.
                            self.peers.ban(&peer);
                            return self.handle_event(
                                effect_builder,
                                rng,
                                Event::GetBlockHeightResult(
                                    block_height,
                                    BlockByHeightResult::Absent(peer),
                                ),
                            );
                        }
                        self.peers.success(peer);
                        self.block_downloaded(rng, effect_builder, &block)
                    }
                }
            }
            Event::GetBlockHashResult(block_hash, fetch_result) => {
                match fetch_result {
                    BlockByHashResult::Absent(peer) => {
                        self.metrics.observe_get_block_by_hash();
                        trace!(
                            %block_hash, %peer,
                            "failed to download block by hash. Trying next peer"
                        );
                        self.peers.failure(&peer);
                        match self.peers.random() {
                            None if self.started_syncing => {
                                error!(
                                    %block_hash,
                                    "could not download linear block from any of the peers."
                                );
                                fatal!(effect_builder, "failed to synchronize linear chain")
                                    .ignore()
                            }
                            None => {
                                warn!(
                                    "run out of peers before managed to start syncing. \
                                    Resetting peers' list and continuing"
                                );
                                self.peers.reset(rng);
                                self.metrics.reset_start_time();
                                fetch_block_by_hash(effect_builder, peer, block_hash)
                            }
                            Some(peer) => {
                                self.metrics.reset_start_time();
                                fetch_block_by_hash(effect_builder, peer, block_hash)
                            }
                        }
                    }
                    BlockByHashResult::FromStorage(block) => {
                        // We shouldn't get invalid data from the storage.
                        // If we do, it's a bug.
                        assert_eq!(*block.hash(), block_hash, "Block hash mismatch.");
                        trace!(%block_hash, "Linear block found in the local storage.");
                        // We hit a block that we already had in the storage - which should mean
                        // that we also have all of its ancestors, so we switch to traversing the
                        // chain forwards and downloading the deploys.
                        // We don't want to download and execute a block we already have, so
                        // instead of calling self.block_downloaded(), we take a shortcut:
                        self.set_last_block_if_syncing_trusted_hash(&block);
                        self.block_handled(rng, effect_builder, *block)
                    }
                    BlockByHashResult::FromPeer(block, peer) => {
                        self.metrics.observe_get_block_by_hash();
                        trace!(%block_hash, %peer, "linear chain block downloaded from a peer");
                        let header_hash = block.header().hash();
                        if header_hash != block_hash || header_hash != *block.hash() {
                            warn!(
                                "Block hash mismatch. Expected {} got {} from {}.\
                                 Block claims to have hash {}. Disconnecting.",
                                block_hash,
                                header_hash,
                                block.hash(),
                                peer
                            );
                            // NOTE: Signal misbehaving validator to networking layer.
                            self.peers.ban(&peer);
                            return self.handle_event(
                                effect_builder,
                                rng,
                                Event::GetBlockHashResult(
                                    block_hash,
                                    BlockByHashResult::Absent(peer),
                                ),
                            );
                        }
                        self.peers.success(peer);
                        self.block_downloaded(rng, effect_builder, &block)
                    }
                }
            }
            Event::GetDeploysResult(fetch_result) => {
                self.metrics.observe_get_deploys();
                match fetch_result {
                    event::DeploysResult::Found(block) => {
                        let block_hash = block.hash();
                        trace!(%block_hash, "deploys for linear chain block found");
                        // Reset used peers so we can download next block with the full set.
                        self.peers.reset(rng);
                        // Execute block
                        let finalized_block: FinalizedBlock = (*block).into();
                        effect_builder.execute_block(finalized_block).ignore()
                    }
                    event::DeploysResult::NotFound(block, peer) => {
                        let block_hash = block.hash();
                        trace!(
                            %block_hash, %peer,
                            "deploy for linear chain block not found. Trying next peer"
                        );
                        self.peers.failure(&peer);
                        match self.peers.random() {
                            None => {
                                error!(
                                    %block_hash,
                                    "could not download deploys from linear chain block."
                                );
                                fatal!(effect_builder, "failed to download linear chain deploys")
                                    .ignore()
                            }
                            Some(peer) => {
                                self.metrics.reset_start_time();
                                fetch_block_deploys(effect_builder, peer, *block)
                            }
                        }
                    }
                }
            }
            Event::StartDownloadingDeploys => {
                // Start downloading deploys from the first block of the linear chain.
                self.peers.reset(rng);
                self.fetch_next_block_deploys(effect_builder)
            }
            Event::NewPeerConnected(peer_id) => {
                trace!(%peer_id, "new peer connected");
                // Add to the set of peers we can request things from.
                self.peers.push(peer_id.clone());
                match self.init_hash {
                    Some(init_hash) if !self.started => {
                        self.started = true;
                        let chainspec = self.chainspec.clone();
                        (async move {
                            match operations::run_fast_sync_task(
                                effect_builder,
                                init_hash,
                                chainspec,
                            )
                            .await
                            {
                                Ok(_block_header) => Some(Event::Start(peer_id)),
                                Err(error) => {
                                    fatal!(effect_builder, "{:?}", error).await;
                                    None
                                }
                            }
                        })
                        .map_some(std::convert::identity)
                    }
                    _ => Effects::new(),
                }
            }
            Event::BlockHandled(block) => {
                let block_height = block.height();
                let block_hash = *block.hash();
                let effects = self.block_handled(rng, effect_builder, *block);
                trace!(%block_height, %block_hash, "block handled");
                effects
            }
            Event::GotUpgradeActivationPoint(next_upgrade_activation_point) => {
                trace!(%next_upgrade_activation_point, "new activation point");
                self.next_upgrade_activation_point = Some(next_upgrade_activation_point);
                Effects::new()
            }
            Event::InitUpgradeShutdown => {
                info!("shutdown initiated");
                // Serialize and store state.
                self.handle_upgrade_shutdown(effect_builder)
            }
            Event::Shutdown(upgrade) => {
                info!(%upgrade, "ready for shutdown");
                self.stop_for_upgrade = upgrade;
                Effects::new()
            }
            Event::InitializeTimeout => {
                if !self.started_syncing {
                    info!("hasn't downloaded any blocks in expected time window. Shutting down…");
                    fatal!(effect_builder, "no syncing progress, shutting down…").ignore()
                } else {
                    Effects::new()
                }
            }
        }
    }
}

fn fetch_block_deploys<I: Debug + Clone + Eq + Send + 'static, REv>(
    effect_builder: EffectBuilder<REv>,
    peer: I,
    block: Block,
) -> Effects<Event<I>>
where
    REv: ReactorEventT<I>,
{
    effect_builder
        .validate_block(peer.clone(), block.clone())
        .event(move |valid| {
            if valid {
                Event::GetDeploysResult(DeploysResult::Found(Box::new(block)))
            } else {
                Event::GetDeploysResult(DeploysResult::NotFound(Box::new(block), peer))
            }
        })
}

fn fetch_block_by_hash<I: Clone + Eq + Debug + Send + 'static, REv>(
    effect_builder: EffectBuilder<REv>,
    peer: I,
    block_hash: BlockHash,
) -> Effects<Event<I>>
where
    REv: ReactorEventT<I>,
{
    async move {
        let peer_to_fetch_from = peer.clone();
        match effect_builder
            .fetch::<Block, I>(block_hash, peer_to_fetch_from)
            .await
        {
            Ok(FetchedData::FromStorage { item }) => Some(Event::GetBlockHashResult(
                block_hash,
                BlockByHashResult::FromStorage(item),
            )),
            Ok(FetchedData::FromPeer { item, peer }) => Some(Event::GetBlockHashResult(
                block_hash,
                BlockByHashResult::FromPeer(item, peer),
            )),
            Err(FetcherError::AbsentFromPeer { peer, .. })
            | Err(FetcherError::TimedOutFromPeer { peer, .. }) => {
                warn!(
                    "Could not get block with hash {:?} from peer {:?}",
                    block_hash, peer
                );
                Some(Event::GetBlockHashResult(
                    block_hash,
                    BlockByHashResult::Absent(peer),
                ))
            }
            Err(error) => {
                warn!(
                    "Error getting hash {:?} from peer {:?}: {}",
                    block_hash, peer, error
                );
                None
            }
        }
    }
    .map_some(std::convert::identity)
}

fn fetch_block_at_height<I: Debug + Eq + Send + Clone + 'static, REv>(
    effect_builder: EffectBuilder<REv>,
    peer: I,
    block_height: u64,
) -> Effects<Event<I>>
where
    REv: ReactorEventT<I>,
{
    async move {
        let peer_to_fetch_from = peer.clone();
        match effect_builder
            .fetch::<BlockWithMetadata, I>(block_height, peer_to_fetch_from)
            .await
        {
            Ok(FetchedData::FromStorage { item }) => Event::GetBlockHeightResult(
                block_height,
                BlockByHeightResult::FromStorage(Box::new(item.block)),
            ),
            Ok(FetchedData::FromPeer { item, peer }) => Event::GetBlockHeightResult(
                block_height,
                BlockByHeightResult::FromPeer(Box::new(item.block), peer),
            ),
            Err(error) => {
                warn!(
                    "Could not get block with height {} from peer {:?}: {}",
                    block_height, peer, error
                );
                Event::GetBlockHeightResult(block_height, BlockByHeightResult::Absent(peer))
            }
        }
    }
    .event(std::convert::identity)
}

/// Returns key in the database, under which the LinearChainSync's state is stored.
fn create_state_key(chainspec: &Chainspec) -> Vec<u8> {
    format!(
        "linear_chain_sync:network_name={}",
        chainspec.network_config.name.clone()
    )
    .into()
}

/// Deserialized vector of bytes into `LinearChainSync::State`.
/// Panics on deserialization errors.
fn deserialize_state(serialized_state: &[u8]) -> Option<State> {
    bincode::deserialize(&serialized_state).unwrap_or_else(|error| {
        // Panicking here should not corrupt the state of any component as it's done in the
        // constructor.
        panic!(
            "could not deserialize state from storage, error {:?}",
            error
        )
    })
}

/// Reads the `LinearChainSync's` state from storage, if any.
/// Panics on deserialization errors.
pub(crate) fn read_init_state(
    storage: &Storage,
    chainspec: &Chainspec,
) -> Result<Option<State>, storage::Error> {
    let key = create_state_key(&chainspec);
    if let Some(bytes) = storage.read_state_store(&key)? {
        Ok(deserialize_state(&bytes))
    } else {
        Ok(None)
    }
}

/// Cleans the linear chain state storage.
/// May fail with storage error.
pub(crate) fn clean_linear_chain_state(
    storage: &Storage,
    chainspec: &Chainspec,
) -> Result<bool, storage::Error> {
    let key = create_state_key(&chainspec);
    storage.del_state_store(key)
}
