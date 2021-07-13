//! Effects subsystem.
//!
//! Effects describe things that the creator of the effect intends to happen, producing a value upon
//! completion. They are, in fact, futures.
//!
//! A pinned, boxed future returning an event is called an effect and typed as an `Effect<Ev>`,
//! where `Ev` is the event's type. Generally, `Ev` is an Event enum defined at the top level of
//! each component in the `crate::components` module.
//!
//! ## Using effects
//!
//! To create an effect, an `EffectBuilder` will be passed in from the relevant reactor. For
//! example, given an effect builder `effect_builder`, we can create a `set_timeout` future and turn
//! it into an effect:
//!
//! ```ignore
//! use std::time::Duration;
//! use casper_node::effect::EffectExt;
//!
//! enum Event {
//!     ThreeSecondsElapsed(Duration)
//! }
//!
//! effect_builder
//!     .set_timeout(Duration::from_secs(3))
//!     .event(Event::ThreeSecondsElapsed);
//! ```
//!
//! This example will produce an effect that, after three seconds, creates an
//! `Event::ThreeSecondsElapsed`. Note that effects do nothing on their own, they need to be passed
//! to a [`reactor`](../reactor/index.html) to be executed.
//!
//! ## Arbitrary effects
//!
//! While it is technically possible to turn any future into an effect, it is advisable to only use
//! the effects explicitly listed in this module through traits to create them. Post-processing on
//! effects to turn them into events should also be kept brief.
//!
//! ## Announcements and effects
//!
//! Some effects can be further classified into either announcements or requests, although these
//! properties are not reflected in the type system.
//!
//! **Announcements** are effects emitted by components that are essentially "fire-and-forget"; the
//! component will never expect an answer for these and does not rely on them being handled. It is
//! also conceivable that they are being cloned and dispatched to multiple components by the
//! reactor.
//!
//! A good example is the arrival of a new deploy passed in by a client. Depending on the setup it
//! may be stored, buffered or, in certain testing setups, just discarded. None of this is a concern
//! of the component that talks to the client and deserializes the incoming deploy though, which
//! considers the deploy no longer its concern after it has returned an announcement effect.
//!
//! **Requests** are complex effects that are used when a component needs something from
//! outside of itself (typically to be provided by another component); a request requires an
//! eventual response.
//!
//! A request **must** have a `Responder` field, which a handler of a request **must** call at
//! some point. Failing to do so will result in a resource leak.

pub mod announcements;
pub mod requests;

use std::{
    any::type_name,
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    fmt::{self, Debug, Display, Formatter},
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use datasize::DataSize;
use futures::{channel::oneshot, future::BoxFuture, FutureExt};
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Serialize};
use smallvec::{smallvec, SmallVec};
use tokio::{sync::Semaphore, time};
use tracing::error;
#[cfg(not(feature = "fast-sync"))]
use tracing::warn;

use casper_execution_engine::{
    core::engine_state::{
        self,
        era_validators::GetEraValidatorsError,
        execution_effect::ExecutionEffect,
        genesis::GenesisSuccess,
        upgrade::{UpgradeConfig, UpgradeSuccess},
        BalanceRequest, BalanceResult, GetBidsRequest, GetBidsResult, QueryRequest, QueryResult,
        MAX_PAYMENT,
    },
    shared::{newtypes::Blake2bHash, stored_value::StoredValue},
    storage::{protocol_data::ProtocolData, trie::Trie},
};
use casper_types::{
    system::auction::EraValidators, EraId, ExecutionResult, Key, ProtocolVersion, PublicKey,
    Transfer, U512,
};

use crate::{
    components::{
        block_validator::ValidatingBlock,
        chainspec_loader::{CurrentRunInfo, NextUpgrade},
        consensus::{BlockContext, ClContext},
        contract_runtime::EraValidatorsRequest,
        deploy_acceptor,
        fetcher::FetchResult,
        small_network::GossipedAddress,
    },
    crypto::hash::Digest,
    reactor::{EventQueueHandle, QueueKind},
    types::{
        Block, BlockByHeight, BlockHash, BlockHeader, BlockPayload, BlockSignatures, Chainspec,
        ChainspecInfo, Deploy, DeployHash, DeployHeader, DeployMetadata, FinalitySignature,
        FinalizedBlock, Item, TimeDiff, Timestamp,
    },
    utils::Source,
};
use announcements::{
    ChainspecLoaderAnnouncement, ConsensusAnnouncement, ContractRuntimeAnnouncement,
    ControlAnnouncement, DeployAcceptorAnnouncement, GossiperAnnouncement, LinearChainAnnouncement,
    NetworkAnnouncement, RpcServerAnnouncement,
};
use requests::{
    BlockPayloadRequest, BlockProposerRequest, BlockValidationRequest, ChainspecLoaderRequest,
    ConsensusRequest, ContractRuntimeRequest, FetcherRequest, MetricsRequest, NetworkInfoRequest,
    NetworkRequest, StateStoreRequest, StorageRequest,
};

use self::announcements::BlocklistAnnouncement;
use crate::components::contract_runtime::{
    BlockAndExecutionEffects, BlockExecutionError, ExecutionPreState,
};

/// A resource that will never be available, thus trying to acquire it will wait forever.
static UNOBTAINABLE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(0));

/// A pinned, boxed future that produces one or more events.
pub type Effect<Ev> = BoxFuture<'static, Multiple<Ev>>;

/// Multiple effects in a container.
pub type Effects<Ev> = Multiple<Effect<Ev>>;

/// A small collection of rarely more than two items.
///
/// Stored in a `SmallVec` to avoid allocations in case there are less than three items grouped. The
/// size of two items is chosen because one item is the most common use case, and large items are
/// typically boxed. In the latter case two pointers and one enum variant discriminator is almost
/// the same size as an empty vec, which is two pointers.
pub type Multiple<T> = SmallVec<[T; 2]>;

/// A responder satisfying a request.
#[must_use]
#[derive(DataSize)]
pub struct Responder<T>(Option<oneshot::Sender<T>>);

impl<T: 'static + Send> Responder<T> {
    /// Creates a new `Responder`.
    #[inline]
    fn new(sender: oneshot::Sender<T>) -> Self {
        Responder(Some(sender))
    }

    /// Helper method for tests.
    ///
    /// Allows creating a responder manually. This function should not be used, unless you are
    /// writing alternative infrastructure, e.g. for tests.
    #[cfg(test)]
    #[inline]
    pub(crate) fn create(sender: oneshot::Sender<T>) -> Self {
        Responder::new(sender)
    }
}

impl<T> Responder<T> {
    /// Send `data` to the origin of the request.
    pub async fn respond(mut self, data: T) {
        if let Some(sender) = self.0.take() {
            if sender.send(data).is_err() {
                error!("could not send response to request down oneshot channel");
            }
        } else {
            error!("tried to send a value down a responder channel, but it was already used");
        }
    }
}

impl<T> Debug for Responder<T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "Responder<{}>", type_name::<T>(),)
    }
}

impl<T> Display for Responder<T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "responder({})", type_name::<T>(),)
    }
}

impl<T> Drop for Responder<T> {
    fn drop(&mut self) {
        if self.0.is_some() {
            // This is usually a very serious error, as another component will now be stuck.
            error!(
                "{} dropped without being responded to --- \
                 this is always a bug and will likely cause another component to be stuck!",
                self
            );
        }
    }
}

impl<T> Serialize for Responder<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{:?}", self))
    }
}

/// Effect extension for futures, used to convert futures into actual effects.
pub trait EffectExt: Future + Send {
    /// Finalizes a future into an effect that returns a single event.
    ///
    /// The function `f` is used to translate the returned value from an effect into an event.
    fn event<U, F>(self, f: F) -> Effects<U>
    where
        F: FnOnce(Self::Output) -> U + 'static + Send,
        U: 'static,
        Self: Sized;

    /// Finalizes a future into an effect that returns an iterator of events.
    ///
    /// The function `f` is used to translate the returned value from an effect into an iterator of
    /// events.
    fn events<U, F, I>(self, f: F) -> Effects<U>
    where
        F: FnOnce(Self::Output) -> I + 'static + Send,
        U: 'static,
        I: Iterator<Item = U>,
        Self: Sized;

    /// Finalizes a future into an effect that runs but drops the result.
    fn ignore<Ev>(self) -> Effects<Ev>;
}

/// Effect extension for futures, used to convert futures returning a `Result` into two different
/// effects.
pub trait EffectResultExt {
    /// The type the future will return if `Ok`.
    type Value;
    /// The type the future will return if `Err`.
    type Error;

    /// Finalizes a future returning a `Result` into two different effects.
    ///
    /// The function `f_ok` is used to translate the returned value from an effect into an event,
    /// while the function `f_err` does the same for a potential error.
    fn result<U, F, G>(self, f_ok: F, f_err: G) -> Effects<U>
    where
        F: FnOnce(Self::Value) -> U + 'static + Send,
        G: FnOnce(Self::Error) -> U + 'static + Send,
        U: 'static;
}

/// Effect extension for futures, used to convert futures returning an `Option` into two different
/// effects.
pub trait EffectOptionExt {
    /// The type the future will return if `Some`.
    type Value;

    /// Finalizes a future returning an `Option` into two different effects.
    ///
    /// The function `f_some` is used to translate the returned value from an effect into an event,
    /// while the function `f_none` does the same for a returned `None`.
    fn map_or_else<U, F, G>(self, f_some: F, f_none: G) -> Effects<U>
    where
        F: FnOnce(Self::Value) -> U + 'static + Send,
        G: FnOnce() -> U + 'static + Send,
        U: 'static;

    /// Finalizes a future returning an `Option` into two different effects.
    ///
    /// The function `f` is used to translate the returned value from an effect into an event,
    /// In the case of `None`, empty vector of effects is returned.
    fn map_some<U, F>(self, f: F) -> Effects<U>
    where
        F: FnOnce(Self::Value) -> U + 'static + Send,
        U: 'static;
}

impl<T: ?Sized> EffectExt for T
where
    T: Future + Send + 'static + Sized,
{
    fn event<U, F>(self, f: F) -> Effects<U>
    where
        F: FnOnce(Self::Output) -> U + 'static + Send,
        U: 'static,
    {
        smallvec![self.map(f).map(|item| smallvec![item]).boxed()]
    }

    fn events<U, F, I>(self, f: F) -> Effects<U>
    where
        F: FnOnce(Self::Output) -> I + 'static + Send,
        U: 'static,
        I: Iterator<Item = U>,
    {
        smallvec![self.map(f).map(|iter| iter.collect()).boxed()]
    }

    fn ignore<Ev>(self) -> Effects<Ev> {
        smallvec![self.map(|_| Multiple::new()).boxed()]
    }
}

impl<T, V, E> EffectResultExt for T
where
    T: Future<Output = Result<V, E>> + Send + 'static + Sized,
    T: ?Sized,
{
    type Value = V;
    type Error = E;

    fn result<U, F, G>(self, f_ok: F, f_err: G) -> Effects<U>
    where
        F: FnOnce(V) -> U + 'static + Send,
        G: FnOnce(E) -> U + 'static + Send,
        U: 'static,
    {
        smallvec![self
            .map(|result| result.map_or_else(f_err, f_ok))
            .map(|item| smallvec![item])
            .boxed()]
    }
}

impl<T, V> EffectOptionExt for T
where
    T: Future<Output = Option<V>> + Send + 'static + Sized,
    T: ?Sized,
{
    type Value = V;

    fn map_or_else<U, F, G>(self, f_some: F, f_none: G) -> Effects<U>
    where
        F: FnOnce(V) -> U + 'static + Send,
        G: FnOnce() -> U + 'static + Send,
        U: 'static,
    {
        smallvec![self
            .map(|option| option.map_or_else(f_none, f_some))
            .map(|item| smallvec![item])
            .boxed()]
    }

    /// Finalizes a future returning an `Option`.
    ///
    /// The function `f` is used to translate the returned value from an effect into an event,
    /// In the case of `None`, empty vector is returned.
    fn map_some<U, F>(self, f: F) -> Effects<U>
    where
        F: FnOnce(Self::Value) -> U + 'static + Send,
        U: 'static,
    {
        smallvec![self
            .map(|option| option
                .map(|el| smallvec![f(el)])
                .unwrap_or_else(|| smallvec![]))
            .boxed()]
    }
}

/// A builder for [`Effect`](type.Effect.html)s.
///
/// Provides methods allowing the creation of effects which need to be scheduled
/// on the reactor's event queue, without giving direct access to this queue.
#[derive(Debug)]
pub struct EffectBuilder<REv: 'static>(EventQueueHandle<REv>);

// Implement `Clone` and `Copy` manually, as `derive` will make it depend on `REv` otherwise.
impl<REv> Clone for EffectBuilder<REv> {
    fn clone(&self) -> Self {
        EffectBuilder(self.0)
    }
}

impl<REv> Copy for EffectBuilder<REv> {}

impl<REv> EffectBuilder<REv> {
    /// Creates a new effect builder.
    pub fn new(event_queue_handle: EventQueueHandle<REv>) -> Self {
        EffectBuilder(event_queue_handle)
    }

    /// Extract the event queue handle out of the effect builder.
    #[cfg(test)]
    pub fn into_inner(self) -> EventQueueHandle<REv> {
        self.0
    }

    /// Performs a request.
    ///
    /// Given a request `Q`, that when completed will yield a result of `T`, produces a future
    /// that will
    ///
    /// 1. create an event to send the request to the respective component (thus `Q: Into<REv>`),
    /// 2. waits for a response and returns it.
    ///
    /// This function is usually only used internally by effects implement on the effects builder,
    /// but IO components may also make use of it.
    pub(crate) async fn make_request<T, Q, F>(self, f: F, queue_kind: QueueKind) -> T
    where
        T: Send + 'static,
        Q: Into<REv>,
        F: FnOnce(Responder<T>) -> Q,
    {
        // Prepare a channel.
        let (sender, receiver) = oneshot::channel();

        // Create response function.
        let responder = Responder::new(sender);

        // Now inject the request event into the event loop.
        let request_event = f(responder).into();
        self.0.schedule(request_event, queue_kind).await;

        match receiver.await {
            Ok(value) => value,
            Err(err) => {
                // The channel should never be closed, ever. If it is, we pretend nothing happened
                // though, instead of crashing.
                error!(%err, ?queue_kind, "request for {} channel closed, this may be a bug? \
                       check if a component is stuck from now on ", type_name::<T>());

                // We cannot produce any value to satisfy the request, so we just abandon this task
                // by waiting on a resource we can never acquire.
                let _ = UNOBTAINABLE.acquire().await;
                panic!("should never obtain unobtainable semaphore");
            }
        }
    }

    /// Run and end effect immediately.
    ///
    /// Can be used to trigger events from effects when combined with `.event`. Do not use this to
    /// "do nothing", as it will still cause a task to be spawned.
    #[inline(always)]
    #[allow(clippy::manual_async_fn)]
    pub fn immediately(self) -> impl Future<Output = ()> + Send {
        // Note: This function is implemented manually without `async` sugar because the `Send`
        // inference seems to not work in all cases otherwise.
        async {}
    }

    /// Reports a fatal error.  Normally called via the `crate::fatal!()` macro.
    ///
    /// Usually causes the node to cease operations quickly and exit/crash.
    //
    // Note: This function is implemented manually without `async` sugar because the `Send`
    // inference seems to not work in all cases otherwise.
    pub async fn fatal(self, file: &'static str, line: u32, msg: String)
    where
        REv: From<ControlAnnouncement>,
    {
        self.0
            .schedule(
                ControlAnnouncement::FatalError { file, line, msg },
                QueueKind::Control,
            )
            .await
    }

    /// Sets a timeout.
    pub(crate) async fn set_timeout(self, timeout: Duration) -> Duration {
        let then = Instant::now();
        time::sleep(timeout).await;
        Instant::now() - then
    }

    /// Retrieve a snapshot of the nodes current metrics formatted as string.
    ///
    /// If an error occurred producing the metrics, `None` is returned.
    pub(crate) async fn get_metrics(self) -> Option<String>
    where
        REv: From<MetricsRequest>,
    {
        self.make_request(
            |responder| MetricsRequest::RenderNodeMetricsText { responder },
            QueueKind::Api,
        )
        .await
    }

    /// Sends a network message.
    ///
    /// The message is queued in "fire-and-forget" fashion, there is no guarantee that the peer
    /// will receive it.
    pub(crate) async fn send_message<I, P>(self, dest: I, payload: P)
    where
        REv: From<NetworkRequest<I, P>>,
    {
        self.make_request(
            |responder| NetworkRequest::SendMessage {
                dest: Box::new(dest),
                payload: Box::new(payload),
                responder,
            },
            QueueKind::Network,
        )
        .await
    }

    /// Broadcasts a network message.
    ///
    /// Broadcasts a network message to all peers connected at the time the message is sent.
    pub async fn broadcast_message<I, P>(self, payload: P)
    where
        REv: From<NetworkRequest<I, P>>,
    {
        self.make_request(
            |responder| NetworkRequest::Broadcast {
                payload: Box::new(payload),
                responder,
            },
            QueueKind::Network,
        )
        .await
    }

    /// Gossips a network message.
    ///
    /// A low-level "gossip" function, selects `count` randomly chosen nodes on the network,
    /// excluding the indicated ones, and sends each a copy of the message.
    ///
    /// Returns the IDs of the chosen nodes.
    pub async fn gossip_message<I, P>(
        self,
        payload: P,
        count: usize,
        exclude: HashSet<I>,
    ) -> HashSet<I>
    where
        REv: From<NetworkRequest<I, P>>,
        I: Send + 'static,
        P: Send,
    {
        self.make_request(
            |responder| NetworkRequest::Gossip {
                payload: Box::new(payload),
                count,
                exclude,
                responder,
            },
            QueueKind::Network,
        )
        .await
    }

    /// Gets connected network peers.
    pub async fn network_peers<I>(self) -> BTreeMap<I, String>
    where
        REv: From<NetworkInfoRequest<I>>,
        I: Send + 'static,
    {
        self.make_request(
            |responder| NetworkInfoRequest::GetPeers { responder },
            QueueKind::Api,
        )
        .await
    }

    /// Announces that a network message has been received.
    pub(crate) async fn announce_message_received<I, P>(self, sender: I, payload: P)
    where
        REv: From<NetworkAnnouncement<I, P>>,
    {
        self.0
            .schedule(
                NetworkAnnouncement::MessageReceived { sender, payload },
                QueueKind::NetworkIncoming,
            )
            .await;
    }

    /// Announces that we should gossip our own public listening address.
    pub(crate) async fn announce_gossip_our_address<I, P>(self, our_address: GossipedAddress)
    where
        REv: From<NetworkAnnouncement<I, P>>,
    {
        self.0
            .schedule(
                NetworkAnnouncement::GossipOurAddress(our_address),
                QueueKind::Regular,
            )
            .await;
    }

    /// Announces that a new peer has connected.
    pub(crate) async fn announce_new_peer<I, P>(self, peer_id: I)
    where
        REv: From<NetworkAnnouncement<I, P>>,
    {
        self.0
            .schedule(
                NetworkAnnouncement::NewPeer(peer_id),
                QueueKind::NetworkIncoming,
            )
            .await;
    }

    /// Announces that a gossiper has received a new item, where the item's ID is the complete item.
    pub(crate) async fn announce_complete_item_received_via_gossip<T: Item>(self, item: T::Id)
    where
        REv: From<GossiperAnnouncement<T>>,
    {
        assert!(
            T::ID_IS_COMPLETE_ITEM,
            "{} must be an item where the ID _is_ the complete item",
            item
        );
        self.0
            .schedule(
                GossiperAnnouncement::NewCompleteItem(item),
                QueueKind::Regular,
            )
            .await;
    }

    /// Announces that the HTTP API server has received a deploy.
    pub(crate) async fn announce_deploy_received(
        self,
        deploy: Box<Deploy>,
        responder: Option<Responder<Result<(), deploy_acceptor::Error>>>,
    ) where
        REv: From<RpcServerAnnouncement>,
    {
        self.0
            .schedule(
                RpcServerAnnouncement::DeployReceived { deploy, responder },
                QueueKind::Api,
            )
            .await;
    }

    /// Announces that a deploy not previously stored has now been accepted and stored.
    pub(crate) fn announce_new_deploy_accepted<I>(
        self,
        deploy: Box<Deploy>,
        source: Source<I>,
    ) -> impl Future<Output = ()>
    where
        REv: From<DeployAcceptorAnnouncement<I>>,
    {
        self.0.schedule(
            DeployAcceptorAnnouncement::AcceptedNewDeploy { deploy, source },
            QueueKind::Regular,
        )
    }

    /// Announces that we have finished gossiping the indicated item.
    pub(crate) async fn announce_finished_gossiping<T>(self, item_id: T::Id)
    where
        REv: From<GossiperAnnouncement<T>>,
        T: Item,
    {
        self.0
            .schedule(
                GossiperAnnouncement::FinishedGossiping(item_id),
                QueueKind::Regular,
            )
            .await;
    }

    /// Announces that an invalid deploy has been received.
    pub(crate) fn announce_invalid_deploy<I>(
        self,
        deploy: Box<Deploy>,
        source: Source<I>,
    ) -> impl Future<Output = ()>
    where
        REv: From<DeployAcceptorAnnouncement<I>>,
    {
        self.0.schedule(
            DeployAcceptorAnnouncement::InvalidDeploy { deploy, source },
            QueueKind::Regular,
        )
    }

    /// Announce new block has been created.
    pub(crate) async fn announce_linear_chain_block(
        self,
        block: Block,
        execution_results: HashMap<DeployHash, (DeployHeader, ExecutionResult)>,
    ) where
        REv: From<ContractRuntimeAnnouncement>,
    {
        self.0
            .schedule(
                ContractRuntimeAnnouncement::linear_chain_block(block, execution_results),
                QueueKind::Regular,
            )
            .await
    }

    /// Announce a committed Step success.
    pub(crate) async fn announce_step_success(
        self,
        era_id: EraId,
        execution_effect: ExecutionEffect,
    ) where
        REv: From<ContractRuntimeAnnouncement>,
    {
        self.0
            .schedule(
                ContractRuntimeAnnouncement::step_success(era_id, (&execution_effect).into()),
                QueueKind::Regular,
            )
            .await
    }

    /// Announce upgrade activation point read.
    pub(crate) async fn announce_upgrade_activation_point_read(self, next_upgrade: NextUpgrade)
    where
        REv: From<ChainspecLoaderAnnouncement>,
    {
        self.0
            .schedule(
                ChainspecLoaderAnnouncement::UpgradeActivationPointRead(next_upgrade),
                QueueKind::Regular,
            )
            .await
    }

    /// Puts the given block into the linear block store.
    pub(crate) async fn put_block_to_storage(self, block: Box<Block>) -> bool
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::PutBlock { block, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested block from the linear block store.
    pub(crate) async fn get_block_from_storage(self, block_hash: BlockHash) -> Option<Block>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlock {
                block_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested block header from the linear block store.
    #[allow(unused)]
    pub(crate) async fn get_block_header_from_storage(
        self,
        block_hash: BlockHash,
    ) -> Option<BlockHeader>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockHeader {
                block_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested signatures for a given block hash.
    pub(crate) async fn get_signatures_from_storage(
        self,
        block_hash: BlockHash,
    ) -> Option<BlockSignatures>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockSignatures {
                block_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Puts the requested finality signatures into storage.
    pub(crate) async fn put_signatures_to_storage(self, signatures: BlockSignatures) -> bool
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::PutBlockSignatures {
                signatures,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested block's transfers from storage.
    pub(crate) async fn get_block_transfers_from_storage(
        self,
        block_hash: BlockHash,
    ) -> Option<Vec<Transfer>>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockTransfers {
                block_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests the block header at the given height.
    pub(crate) async fn get_block_header_at_height_from_storage(
        self,
        height: u64,
    ) -> Option<BlockHeader>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockHeaderAtHeight { height, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests the header of the block containing the given deploy.
    pub(crate) async fn get_block_header_for_deploy_from_storage(
        self,
        deploy_hash: DeployHash,
    ) -> Option<BlockHeader>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockHeaderForDeploy {
                deploy_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests the block at the given height.
    pub(crate) async fn get_block_at_height_from_storage(self, height: u64) -> Option<Block>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockAtHeight { height, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests the highest block.
    pub(crate) async fn get_highest_block_from_storage(self) -> Option<Block>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetHighestBlock { responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests the header of the switch block at the given era ID.
    pub(crate) async fn get_switch_block_header_at_era_id_from_storage(
        self,
        era_id: EraId,
    ) -> Option<BlockHeader>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetSwitchBlockHeaderAtEraId { era_id, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests the key block header for the given era ID, ie. the header of the switch block at
    /// the era before (if one exists).
    pub(crate) async fn get_key_block_header_for_era_id_from_storage(
        self,
        era_id: EraId,
    ) -> Option<BlockHeader>
    where
        REv: From<StorageRequest>,
    {
        let era_before = era_id.checked_sub(1)?;
        self.get_switch_block_header_at_era_id_from_storage(era_before)
            .await
    }

    /// Requests the highest switch block.
    // TODO - remove once used.
    #[allow(unused)]
    pub(crate) async fn get_highest_switch_block_from_storage(self) -> Option<Block>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetHighestSwitchBlock { responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Read a trie by its hash key
    pub(crate) async fn read_trie(self, trie_key: Blake2bHash) -> Option<Trie<Key, StoredValue>>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::ReadTrie {
                trie_key,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Puts a trie into the trie store and asynchronously returns any missing descendant trie keys.
    #[allow(unused)]
    pub(crate) async fn put_trie_and_find_missing_descendant_trie_keys(
        self,
        trie: Box<Trie<Key, StoredValue>>,
    ) -> Result<Vec<Blake2bHash>, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::PutTrie { trie, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Puts the given deploy into the deploy store.
    pub(crate) async fn put_deploy_to_storage(self, deploy: Box<Deploy>) -> bool
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::PutDeploy { deploy, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested deploys from the deploy store.
    pub(crate) async fn get_deploys_from_storage(
        self,
        deploy_hashes: Multiple<DeployHash>,
    ) -> Vec<Option<Deploy>>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetDeploys {
                deploy_hashes: deploy_hashes.to_vec(),
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Stores the given execution results for the deploys in the given block in the linear block
    /// store.
    pub(crate) async fn put_execution_results_to_storage(
        self,
        block_hash: BlockHash,
        execution_results: HashMap<DeployHash, ExecutionResult>,
    ) where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::PutExecutionResults {
                block_hash: Box::new(block_hash),
                execution_results,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested deploys from the deploy store.
    pub(crate) async fn get_deploy_and_metadata_from_storage(
        self,
        deploy_hash: DeployHash,
    ) -> Option<(Deploy, DeployMetadata)>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetDeployAndMetadata {
                deploy_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested block and its associated metadata.
    pub(crate) async fn get_block_at_height_with_metadata_from_storage(
        self,
        block_height: u64,
    ) -> Option<(Block, BlockSignatures)>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockAndMetadataByHeight {
                block_height,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested block by hash with its associated metadata.
    pub(crate) async fn get_block_with_metadata_from_storage(
        self,
        block_hash: BlockHash,
    ) -> Option<(Block, BlockSignatures)>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetBlockAndMetadataByHash {
                block_hash,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Get the highest block with its associated metadata.
    pub(crate) async fn get_highest_block_with_metadata_from_storage(
        self,
    ) -> Option<(Block, BlockSignatures)>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            |responder| StorageRequest::GetHighestBlockWithMetadata { responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested deploy using the `DeployFetcher`.
    pub(crate) async fn fetch_deploy<I>(
        self,
        deploy_hash: DeployHash,
        peer: I,
    ) -> Option<FetchResult<Deploy, I>>
    where
        REv: From<FetcherRequest<I, Deploy>>,
        I: Send + 'static,
    {
        self.make_request(
            |responder| FetcherRequest::Fetch {
                id: deploy_hash,
                peer,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested block using the `BlockFetcher`
    pub(crate) async fn fetch_block<I>(
        self,
        block_hash: BlockHash,
        peer: I,
    ) -> Option<FetchResult<Block, I>>
    where
        REv: From<FetcherRequest<I, Block>>,
        I: Send + 'static,
    {
        self.make_request(
            |responder| FetcherRequest::Fetch {
                id: block_hash,
                peer,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests a linear chain block at `block_height`.
    pub(crate) async fn fetch_block_by_height<I>(
        self,
        block_height: u64,
        peer: I,
    ) -> Option<FetchResult<BlockByHeight, I>>
    where
        REv: From<FetcherRequest<I, BlockByHeight>>,
        I: Send + 'static,
    {
        self.make_request(
            |responder| FetcherRequest::Fetch {
                id: block_height,
                peer,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Passes the timestamp of a future block for which deploys are to be proposed.
    pub(crate) async fn request_block_payload(
        self,
        context: BlockContext<ClContext>,
        next_finalized: u64,
        accusations: Vec<PublicKey>,
        random_bit: bool,
    ) -> Arc<BlockPayload>
    where
        REv: From<BlockProposerRequest>,
    {
        self.make_request(
            |responder| {
                BlockProposerRequest::RequestBlockPayload(BlockPayloadRequest {
                    context,
                    next_finalized,
                    responder,
                    accusations,
                    random_bit,
                })
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Executes a finalized block.
    pub(crate) async fn execute_finalized_block(
        self,
        protocol_version: ProtocolVersion,
        execution_pre_state: ExecutionPreState,
        finalized_block: FinalizedBlock,
        deploys: Vec<Deploy>,
    ) -> Result<BlockAndExecutionEffects, BlockExecutionError>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::ExecuteBlock {
                protocol_version,
                execution_pre_state,
                finalized_block,
                deploys,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Enqueues a finalized proto-block execution.
    ///
    /// # Arguments
    ///
    /// * `finalized_block` - a finalized proto-block to add to the execution queue.
    /// * `deploys` - a vector of deploys and transactions that match the hashes in the finalized
    ///   block, in that order.
    pub(crate) async fn enqueue_block_for_execution(
        self,
        finalized_block: FinalizedBlock,
        deploys: Vec<Deploy>,
    ) where
        REv: From<ContractRuntimeRequest>,
    {
        self.0
            .schedule(
                ContractRuntimeRequest::EnqueueBlockForExecution {
                    finalized_block,
                    deploys,
                },
                QueueKind::Regular,
            )
            .await
    }

    /// Checks whether the deploys included in the block exist on the network and the block is
    /// valid.
    pub(crate) async fn validate_block<I, T>(self, sender: I, block: T) -> bool
    where
        REv: From<BlockValidationRequest<I>>,
        T: Into<ValidatingBlock>,
    {
        self.make_request(
            |responder| BlockValidationRequest {
                block: block.into(),
                sender,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Announces that a block has been finalized.
    pub(crate) async fn announce_finalized_block(self, finalized_block: FinalizedBlock)
    where
        REv: From<ConsensusAnnouncement>,
    {
        self.0
            .schedule(
                ConsensusAnnouncement::Finalized(Box::new(finalized_block)),
                QueueKind::Regular,
            )
            .await
    }

    /// Announces that a finality signature has been created.
    pub(crate) async fn announce_created_finality_signature(
        self,
        finality_signature: FinalitySignature,
    ) where
        REv: From<ConsensusAnnouncement>,
    {
        self.0
            .schedule(
                ConsensusAnnouncement::CreatedFinalitySignature(Box::new(finality_signature)),
                QueueKind::Regular,
            )
            .await
    }

    /// An equivocation has been detected.
    pub(crate) async fn announce_fault_event(
        self,
        era_id: EraId,
        public_key: PublicKey,
        timestamp: Timestamp,
    ) where
        REv: From<ConsensusAnnouncement>,
    {
        self.0
            .schedule(
                ConsensusAnnouncement::Fault {
                    era_id,
                    public_key: Box::new(public_key),
                    timestamp,
                },
                QueueKind::Regular,
            )
            .await
    }

    /// Announce the intent to disconnect from a specific peer, which consensus thinks is faulty.
    pub(crate) async fn announce_disconnect_from_peer<I>(self, peer: I)
    where
        REv: From<BlocklistAnnouncement<I>>,
    {
        self.0
            .schedule(
                BlocklistAnnouncement::OffenseCommitted(Box::new(peer)),
                QueueKind::Regular,
            )
            .await
    }

    /// The linear chain has stored a newly-created block.
    pub(crate) async fn announce_block_added(self, block: Box<Block>)
    where
        REv: From<LinearChainAnnouncement>,
    {
        self.0
            .schedule(
                LinearChainAnnouncement::BlockAdded(block),
                QueueKind::Regular,
            )
            .await
    }

    /// The linear chain has stored a new finality signature.
    pub(crate) async fn announce_finality_signature(self, fs: Box<FinalitySignature>)
    where
        REv: From<LinearChainAnnouncement>,
    {
        self.0
            .schedule(
                LinearChainAnnouncement::NewFinalitySignature(fs),
                QueueKind::Regular,
            )
            .await
    }

    /// Runs the genesis process on the contract runtime.
    pub(crate) async fn commit_genesis(
        self,
        chainspec: Arc<Chainspec>,
    ) -> Result<GenesisSuccess, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::CommitGenesis {
                chainspec,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Runs the upgrade process on the contract runtime.
    pub(crate) async fn upgrade_contract_runtime(
        self,
        upgrade_config: Box<UpgradeConfig>,
    ) -> Result<UpgradeSuccess, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::Upgrade {
                upgrade_config,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the requested chainspec info from the chainspec loader.
    pub(crate) async fn get_chainspec_info(self) -> ChainspecInfo
    where
        REv: From<ChainspecLoaderRequest> + Send,
    {
        self.make_request(ChainspecLoaderRequest::GetChainspecInfo, QueueKind::Regular)
            .await
    }

    /// Gets the information about the current run of the node software.
    pub(crate) async fn get_current_run_info(self) -> CurrentRunInfo
    where
        REv: From<ChainspecLoaderRequest>,
    {
        self.make_request(
            ChainspecLoaderRequest::GetCurrentRunInfo,
            QueueKind::Regular,
        )
        .await
    }

    /// Loads potentially previously stored state from storage.
    ///
    /// Key must be a unique key across the the application, as all keys share a common namespace.
    ///
    /// If an error occurs during state loading or no data is found, returns `None`.
    #[allow(unused)]
    pub(crate) async fn load_state<T>(self, key: Cow<'static, [u8]>) -> Option<T>
    where
        REv: From<StateStoreRequest>,
        T: DeserializeOwned,
    {
        // There is an ugly truth hidden in here: Due to object safety issues, we cannot ship the
        // actual values around, but only the serialized bytes. For this reason this function
        // retrieves raw bytes from storage and perform deserialization here.
        //
        // Errors are prominently logged but not treated further in any way.
        self.make_request(
            move |responder| StateStoreRequest::Load { key, responder },
            QueueKind::Regular,
        )
        .await
        .map(|data| bincode::deserialize(&data))
        .transpose()
        .unwrap_or_else(|err| {
            let type_name = type_name::<T>();
            panic!(
                "could not deserialize state from storage type name {:?} err {:?}",
                type_name, err
            );
        })
    }

    /// Retrieves finalized deploys from blocks that were created more recently than the TTL.
    pub(crate) async fn get_finalized_deploys(
        self,
        ttl: TimeDiff,
    ) -> Vec<(DeployHash, DeployHeader)>
    where
        REv: From<StorageRequest>,
    {
        self.make_request(
            move |responder| StorageRequest::GetFinalizedDeploys { ttl, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Save state to storage.
    ///
    /// Key must be a unique key across the the application, as all keys share a common namespace.
    ///
    /// Returns whether or not storing the state was successful. A component that requires state to
    /// be successfully stored should check the return value and act accordingly.
    #[cfg(not(feature = "fast-sync"))]
    pub(crate) async fn save_state<T>(self, key: Cow<'static, [u8]>, value: T) -> bool
    where
        REv: From<StateStoreRequest>,
        T: Serialize,
    {
        match bincode::serialize(&value) {
            Ok(data) => {
                self.make_request(
                    move |responder| StateStoreRequest::Save {
                        key,
                        data,
                        responder,
                    },
                    QueueKind::Regular,
                )
                .await;
                true
            }
            Err(err) => {
                let type_name = type_name::<T>();
                warn!(%type_name, %err, "Error serializing state");
                false
            }
        }
    }

    /// Requests a query be executed on the Contract Runtime component.
    pub(crate) async fn query_global_state(
        self,
        query_request: QueryRequest,
    ) -> Result<QueryResult, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::Query {
                query_request,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    pub(crate) async fn is_verified_account(self, account_key: Key) -> Option<bool>
    where
        REv: From<ContractRuntimeRequest>,
        REv: From<StorageRequest>,
    {
        if let Some(block) = self.get_highest_block_from_storage().await {
            let state_hash = (*block.state_root_hash()).into();
            let query_request = QueryRequest::new(state_hash, account_key, vec![]);
            if let Ok(QueryResult::Success { value, .. }) =
                self.query_global_state(query_request).await
            {
                if let StoredValue::Account(account) = *value {
                    let purse_uref = account.main_purse();
                    let balance_request = BalanceRequest::new(state_hash, purse_uref);
                    if let Ok(balance_result) = self.get_balance(balance_request).await {
                        if let Some(motes) = balance_result.motes() {
                            return Some(motes >= &*MAX_PAYMENT);
                        }
                    }
                }
            }
        }
        None
    }

    /// Requests a query be executed on the Contract Runtime component.
    pub(crate) async fn get_balance(
        self,
        balance_request: BalanceRequest,
    ) -> Result<BalanceResult, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::GetBalance {
                balance_request,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Returns `ProtocolData` by `ProtocolVersion`.
    ///
    /// This operation is read only.
    pub(crate) async fn get_protocol_data(
        self,
        protocol_version: ProtocolVersion,
    ) -> Result<Option<Box<ProtocolData>>, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::GetProtocolData {
                protocol_version,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Returns a map of validators weights for all eras as known from `root_hash`.
    ///
    /// This operation is read only.
    pub(crate) async fn get_era_validators_from_contract_runtime(
        self,
        request: EraValidatorsRequest,
    ) -> Result<EraValidators, GetEraValidatorsError>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::GetEraValidators { request, responder },
            QueueKind::Regular,
        )
        .await
    }

    /// Requests a query be executed on the Contract Runtime component.
    pub(crate) async fn get_bids(
        self,
        get_bids_request: GetBidsRequest,
    ) -> Result<GetBidsResult, engine_state::Error>
    where
        REv: From<ContractRuntimeRequest>,
    {
        self.make_request(
            |responder| ContractRuntimeRequest::GetBids {
                get_bids_request,
                responder,
            },
            QueueKind::Regular,
        )
        .await
    }

    /// Gets the correct era validators set for the given era.
    /// Takes emergency restarts into account based on the information from the chainspec loader.
    pub(crate) async fn get_era_validators(self, era_id: EraId) -> Option<BTreeMap<PublicKey, U512>>
    where
        REv: From<ContractRuntimeRequest> + From<StorageRequest> + From<ChainspecLoaderRequest>,
    {
        let CurrentRunInfo {
            protocol_version,
            initial_state_root_hash,
            last_emergency_restart,
            ..
        } = self.get_current_run_info().await;
        let cutoff_era_id = last_emergency_restart.unwrap_or_else(|| EraId::new(0));
        if era_id < cutoff_era_id {
            // we don't support getting the validators from before the last emergency restart
            return None;
        }
        if era_id == cutoff_era_id {
            // in the activation era, we read the validators from the global state; we use the
            // global state hash of the first block in the era, if it exists - if we can't get it,
            // we use the initial_state_root_hash passed from the chainspec loader
            let root_hash = if era_id.is_genesis() {
                // genesis era - use block at height 0
                self.get_block_header_at_height_from_storage(0)
                    .await
                    .map(|hdr| *hdr.state_root_hash())
                    .unwrap_or(initial_state_root_hash)
            } else {
                // non-genesis - calculate the height based on the key block
                let maybe_key_block_header = self
                    .get_key_block_header_for_era_id_from_storage(era_id)
                    .await;
                // this has to be a match because `Option::and_then` can't deal with async closures
                match maybe_key_block_header {
                    None => None,
                    Some(kb_hdr) => {
                        self.get_block_header_at_height_from_storage(kb_hdr.height() + 1)
                            .await
                    }
                }
                // default to the initial_state_root_hash if there was no key block or no block
                // above the key block for the era
                .map_or(initial_state_root_hash, |hdr| *hdr.state_root_hash())
            };
            let req = EraValidatorsRequest::new(root_hash.into(), protocol_version);
            self.get_era_validators_from_contract_runtime(req)
                .await
                .ok()
                .and_then(|era_validators| era_validators.get(&era_id).cloned())
        } else {
            // in other eras, we just use the validators from the key block
            let key_block_result = self
                .get_key_block_header_for_era_id_from_storage(era_id)
                .await
                .and_then(|kb_hdr| kb_hdr.next_era_validator_weights().cloned());
            if key_block_result.is_some() {
                // if the key block read was successful, just return it
                key_block_result
            } else {
                // if there was no key block, we might be looking at a future era - in such a case,
                // read the state root hash from the highest block and check with the contract
                // runtime
                let highest_block = self.get_highest_block_from_storage().await?;
                let req = EraValidatorsRequest::new(
                    (*highest_block.header().state_root_hash()).into(),
                    protocol_version,
                );
                self.get_era_validators_from_contract_runtime(req)
                    .await
                    .ok()
                    .and_then(|era_validators| era_validators.get(&era_id).cloned())
            }
        }
    }

    /// Checks whether the given validator is bonded in the given era.
    pub(crate) async fn is_bonded_validator(
        self,
        validator: PublicKey,
        era_id: EraId,
        latest_state_root_hash: Option<Digest>,
        protocol_version: ProtocolVersion,
    ) -> Result<bool, GetEraValidatorsError>
    where
        REv: From<ContractRuntimeRequest> + From<StorageRequest> + From<ChainspecLoaderRequest>,
    {
        // try just reading the era validators first
        let maybe_era_validators = self.get_era_validators(era_id).await;
        let maybe_is_currently_bonded =
            maybe_era_validators.map(|validators| validators.contains_key(&validator));

        match maybe_is_currently_bonded {
            // if we know whether the validator is bonded, just return that
            Some(is_bonded) => Ok(is_bonded),
            // if not, try checking future eras with the latest state root hash
            None => match latest_state_root_hash {
                // no root hash later than initial -> we just assume the validator is not bonded
                None => Ok(false),
                // otherwise, check with contract runtime
                Some(state_root_hash) => self
                    .make_request(
                        |responder| ContractRuntimeRequest::IsBonded {
                            state_root_hash,
                            era_id,
                            protocol_version,
                            public_key: validator,
                            responder,
                        },
                        QueueKind::Regular,
                    )
                    .await
                    .or_else(|error| {
                        // Promote this error to a non-error case.
                        // It's not an error that we can't find the era that was requested.
                        if error.is_era_validators_missing() {
                            Ok(false)
                        } else {
                            Err(error)
                        }
                    }),
            },
        }
    }

    /// Get our public key from consensus, and if we're a validator, the next round length.
    pub(crate) async fn consensus_status(self) -> Option<(PublicKey, Option<TimeDiff>)>
    where
        REv: From<ConsensusRequest>,
    {
        self.make_request(ConsensusRequest::Status, QueueKind::Regular)
            .await
    }

    /// Collects the key blocks for the eras identified by provided era IDs. Returns
    /// `Some(HashMap(era_id → block_header))` if all the blocks have been read correctly, and
    /// `None` if at least one was missing. The header for EraId `n` is from the key block for that
    /// era, that is, the switch block of era `n-1`, ie. it contains the data necessary for
    /// initialization of era `n`.
    pub(crate) async fn collect_key_block_headers<I: IntoIterator<Item = EraId>>(
        self,
        era_ids: I,
    ) -> Option<HashMap<EraId, BlockHeader>>
    where
        REv: From<StorageRequest>,
    {
        futures::future::join_all(
            era_ids
                .into_iter()
                // we would get None for era 0 and that would make it seem like the entire
                // function failed
                .filter(|era_id| !era_id.is_genesis())
                .map(|era_id| {
                    self.get_key_block_header_for_era_id_from_storage(era_id)
                        .map(move |maybe_header| {
                            maybe_header.map(|block_header| (era_id, block_header))
                        })
                }),
        )
        .await
        .into_iter()
        .collect()
    }
}

/// Construct a fatal error effect.
///
/// This macro is a convenient wrapper around `EffectBuilder::fatal` that inserts the `file!()` and
/// `line!()` number automatically.
#[macro_export]
macro_rules! fatal {
    ($effect_builder:expr, $($arg:tt)*) => {
        $effect_builder.fatal(file!(), line!(), format!($($arg)*))
    };
}
