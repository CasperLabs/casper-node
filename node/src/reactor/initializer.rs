//! Reactor used to initialize a node.

use std::fmt::{self, Display, Formatter};

use datasize::DataSize;
use derive_more::From;
use prometheus::Registry;
use reactor::ReactorEvent;
use serde::Serialize;
use thiserror::Error;
use tracing::info;

use crate::{
    components::{
        chainspec_loader::{self, ChainspecLoader},
        contract_runtime::{self, ContractRuntime},
        gossiper,
        network::NetworkIdentity,
        small_network::{GossipedAddress, SmallNetworkIdentity, SmallNetworkIdentityError},
        storage::{self, Storage},
        Component,
    },
    effect::{
        announcements::{
            ChainspecLoaderAnnouncement, ContractRuntimeAnnouncement, ControlAnnouncement,
        },
        requests::{
            ConsensusRequest, ContractRuntimeRequest, LinearChainRequest, NetworkRequest,
            RestRequest, StateStoreRequest, StorageRequest,
        },
        EffectBuilder, Effects,
    },
    protocol::Message,
    reactor::{self, participating, EventQueueHandle, ReactorExit},
    types::{chainspec, NodeId},
    utils::WithDir,
    NodeRng,
};

/// Top-level event for the reactor.
#[derive(Debug, From, Serialize)]
#[must_use]
pub enum Event {
    /// Chainspec handler event.
    #[from]
    Chainspec(chainspec_loader::Event),

    /// Storage event.

    #[from]
    Storage(#[serde(skip_serializing)] storage::Event),

    /// Contract runtime event.
    ContractRuntime(#[serde(skip_serializing)] Box<ContractRuntimeRequest>),

    /// Request for state storage.
    #[from]
    StateStoreRequest(StateStoreRequest),

    /// Control announcement
    #[from]
    ControlAnnouncement(ControlAnnouncement),
}

impl From<ContractRuntimeRequest> for Event {
    fn from(contract_runtime_request: ContractRuntimeRequest) -> Self {
        Event::ContractRuntime(Box::new(contract_runtime_request))
    }
}

impl ReactorEvent for Event {
    fn as_control(&self) -> Option<&ControlAnnouncement> {
        if let Self::ControlAnnouncement(ref ctrl_ann) = self {
            Some(ctrl_ann)
        } else {
            None
        }
    }
}

impl From<StorageRequest> for Event {
    fn from(request: StorageRequest) -> Self {
        Event::Storage(storage::Event::StorageRequest(request))
    }
}

impl From<NetworkRequest<NodeId, Message>> for Event {
    fn from(_request: NetworkRequest<NodeId, Message>) -> Self {
        unreachable!("no network traffic happens during initialization")
    }
}

impl From<ChainspecLoaderAnnouncement> for Event {
    fn from(_announcement: ChainspecLoaderAnnouncement) -> Self {
        unreachable!("no chainspec announcements happen during initialization")
    }
}

impl From<LinearChainRequest<NodeId>> for Event {
    fn from(_req: LinearChainRequest<NodeId>) -> Self {
        unreachable!("no linear chain events happen during initialization")
    }
}

impl From<NetworkRequest<NodeId, gossiper::Message<GossipedAddress>>> for Event {
    fn from(_request: NetworkRequest<NodeId, gossiper::Message<GossipedAddress>>) -> Self {
        unreachable!("no gossiper events happen during initialization")
    }
}

impl From<ConsensusRequest> for Event {
    fn from(_request: ConsensusRequest) -> Self {
        unreachable!("no chainspec announcements happen during initialization")
    }
}

impl From<RestRequest<NodeId>> for Event {
    fn from(_request: RestRequest<NodeId>) -> Self {
        unreachable!("no rest requests happen during initialization")
    }
}

impl From<ContractRuntimeAnnouncement> for Event {
    fn from(_request: ContractRuntimeAnnouncement) -> Self {
        unreachable!("no block executor requests happen during initialization")
    }
}

impl Display for Event {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Chainspec(event) => write!(formatter, "chainspec: {}", event),
            Event::Storage(event) => write!(formatter, "storage: {}", event),
            Event::ContractRuntime(event) => write!(formatter, "contract runtime: {:?}", event),
            Event::StateStoreRequest(request) => {
                write!(formatter, "state store request: {}", request)
            }
            Event::ControlAnnouncement(ctrl_ann) => write!(formatter, "control: {}", ctrl_ann),
        }
    }
}

/// Error type returned by the initializer reactor.
#[derive(Debug, Error)]
pub enum Error {
    /// `Config` error.
    #[error("config error: {0}")]
    ConfigError(String),

    /// Metrics-related error
    #[error("prometheus (metrics) error: {0}")]
    Metrics(#[from] prometheus::Error),

    /// `ChainspecHandler` component error.
    #[error("chainspec error: {0}")]
    Chainspec(#[from] chainspec::Error),

    /// `Storage` component error.
    #[error("storage error: {0}")]
    Storage(#[from] storage::Error),

    /// `ContractRuntime` component error.
    #[error("contract runtime config error: {0}")]
    ContractRuntime(#[from] contract_runtime::ConfigError),

    /// An error that occurred when creating a `SmallNetworkIdentity`.
    #[error(transparent)]
    SmallNetworkIdentityError(#[from] SmallNetworkIdentityError),
}

/// Initializer node reactor.
#[derive(DataSize, Debug)]
pub struct Reactor {
    pub(super) config: WithDir<participating::Config>,
    pub(super) chainspec_loader: ChainspecLoader,
    pub(super) storage: Storage,
    pub(super) contract_runtime: ContractRuntime,
    pub(super) small_network_identity: SmallNetworkIdentity,
    #[data_size(skip)]
    pub(super) network_identity: NetworkIdentity,
}

impl Reactor {
    fn new_with_chainspec_loader(
        (crashed, config): <Self as reactor::Reactor>::Config,
        registry: &Registry,
        chainspec_loader: ChainspecLoader,
        chainspec_effects: Effects<chainspec_loader::Event>,
    ) -> Result<(Self, Effects<Event>), Error> {
        let hard_reset_to_start_of_era = chainspec_loader.hard_reset_to_start_of_era();

        let storage_config = config.map_ref(|cfg| cfg.storage.clone());
        let storage = Storage::new(
            &storage_config,
            hard_reset_to_start_of_era,
            chainspec_loader.chainspec().protocol_config.version,
            crashed,
        )?;

        let contract_runtime = ContractRuntime::new(
            chainspec_loader.chainspec().protocol_config.version,
            storage_config,
            &config.value().contract_runtime,
            registry,
        )?;

        // TODO: This integrity check is misplaced, it should be part of the components
        // `handle_event` function. Ideally it would be in the constructor, but since a query to
        // storage needs to be made, this is not possible.
        //
        // Refactoring this has been postponed for now, since it is unclear whether time-consuming
        // integrity checks are even a good idea, as they can block the node for one or more hours
        // on restarts (online checks are an alternative).
        if crashed {
            info!("running trie-store integrity check, this may take a while");
            if let Some(state_roots) = storage.get_state_root_hashes_for_trie_check() {
                let missing_trie_keys = contract_runtime.trie_store_check(state_roots.clone());
                if !missing_trie_keys.is_empty() {
                    panic!(
                        "Fatal error! Trie-Key store is not empty.\n {:?}\n \
                        Wipe the DB to ensure operations.\n Present state_roots: {:?}",
                        missing_trie_keys, state_roots
                    )
                }
            }
        }

        let effects = reactor::wrap_effects(Event::Chainspec, chainspec_effects);

        let small_network_identity = SmallNetworkIdentity::new()?;

        let network_identity = NetworkIdentity::new();

        let reactor = Reactor {
            config,
            chainspec_loader,
            storage,
            contract_runtime,
            small_network_identity,
            network_identity,
        };
        Ok((reactor, effects))
    }
}

#[cfg(test)]
impl Reactor {
    /// Inspect storage.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }
}

impl reactor::Reactor for Reactor {
    type Event = Event;
    type Config = (bool, WithDir<participating::Config>);
    type Error = Error;

    fn new(
        config: Self::Config,
        registry: &Registry,
        event_queue: EventQueueHandle<Self::Event>,
        _rng: &mut NodeRng,
    ) -> Result<(Self, Effects<Self::Event>), Error> {
        let effect_builder = EffectBuilder::new(event_queue);

        // Construct the `ChainspecLoader` first so we fail fast if the chainspec is invalid.
        let (chainspec_loader, chainspec_effects) =
            ChainspecLoader::new(config.1.dir(), effect_builder)?;
        Self::new_with_chainspec_loader(config, registry, chainspec_loader, chainspec_effects)
    }

    fn dispatch_event(
        &mut self,
        effect_builder: EffectBuilder<Self::Event>,
        rng: &mut NodeRng,
        event: Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Chainspec(event) => reactor::wrap_effects(
                Event::Chainspec,
                self.chainspec_loader
                    .handle_event(effect_builder, rng, event),
            ),
            Event::Storage(event) => reactor::wrap_effects(
                Event::Storage,
                self.storage.handle_event(effect_builder, rng, event),
            ),
            Event::ContractRuntime(event) => reactor::wrap_effects(
                Event::from,
                self.contract_runtime
                    .handle_event(effect_builder, rng, *event),
            ),
            Event::StateStoreRequest(request) => {
                self.dispatch_event(effect_builder, rng, Event::Storage(request.into()))
            }
            Event::ControlAnnouncement(_) => unreachable!("unhandled control announcement"),
        }
    }

    fn maybe_exit(&self) -> Option<ReactorExit> {
        self.chainspec_loader.reactor_exit()
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::{
        components::network::ENABLE_LIBP2P_NET_ENV_VAR, testing::network::NetworkedReactor,
        types::Chainspec,
    };
    use std::{env, sync::Arc};

    impl Reactor {
        pub(crate) fn new_with_chainspec(
            config: <Self as reactor::Reactor>::Config,
            registry: &Registry,
            event_queue: EventQueueHandle<Event>,
            chainspec: Arc<Chainspec>,
        ) -> Result<(Self, Effects<Event>), Error> {
            let effect_builder = EffectBuilder::new(event_queue);
            let (chainspec_loader, chainspec_effects) =
                ChainspecLoader::new_with_chainspec(chainspec, effect_builder);
            Self::new_with_chainspec_loader(config, registry, chainspec_loader, chainspec_effects)
        }
    }

    impl NetworkedReactor for Reactor {
        type NodeId = NodeId;
        fn node_id(&self) -> Self::NodeId {
            if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_err() {
                NodeId::from(&self.small_network_identity)
            } else {
                NodeId::from(&self.network_identity)
            }
        }
    }
}
