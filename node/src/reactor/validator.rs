//! Reactor for validator nodes.
//!
//! Validator nodes join the validator-only network upon startup.

mod config;
mod error;

use std::fmt::{self, Display, Formatter};

use derive_more::From;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    components::{
        api_server::{self, ApiServer},
        consensus::{self, EraSupervisor},
        contract_runtime::{self, ContractRuntime},
        deploy_buffer::{self, DeployBuffer},
        deploy_gossiper::{self, DeployGossiper},
        pinger::{self, Pinger},
        storage::{Storage, StorageType},
        Component,
    },
    effect::{
        announcements::{ApiServerAnnouncement, NetworkAnnouncement, StorageAnnouncement},
        requests::{
            ApiRequest, ContractRuntimeRequest, DeployQueueRequest, NetworkRequest, StorageRequest,
        },
        EffectBuilder, Effects,
    },
    reactor::{self, initializer, EventQueueHandle},
    small_network::{self, NodeId},
    types::Timestamp,
    SmallNetwork,
};
pub use config::Config;
use error::Error;

/// Reactor message.
#[derive(Debug, Clone, From, Serialize, Deserialize)]
pub enum Message {
    /// Pinger component message.
    #[from]
    Pinger(pinger::Message),
    /// Consensus component message.
    #[from]
    Consensus(consensus::ConsensusMessage),
    /// Deploy gossiper component message.
    #[from]
    DeployGossiper(deploy_gossiper::Message),
}

impl Display for Message {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Message::Pinger(pinger) => write!(f, "Pinger::{}", pinger),
            Message::Consensus(consensus) => write!(f, "Consensus::{}", consensus),
            Message::DeployGossiper(deploy) => write!(f, "DeployGossiper::{}", deploy),
        }
    }
}

/// Top-level event for the reactor.
#[derive(Debug, From)]
#[must_use]
pub enum Event {
    /// Network event.
    #[from]
    Network(small_network::Event<Message>),
    /// Deploy queue event.
    #[from]
    DeployQueue(deploy_buffer::Event),
    /// Pinger event.
    #[from]
    Pinger(pinger::Event),
    #[from]
    /// Storage event.
    Storage(StorageRequest<Storage>),
    #[from]
    /// API server event.
    ApiServer(api_server::Event),
    #[from]
    /// Consensus event.
    Consensus(consensus::Event<NodeId>),
    /// Deploy gossiper event.
    #[from]
    DeployGossiper(deploy_gossiper::Event),
    /// Contract runtime event.
    #[from]
    ContractRuntime(contract_runtime::Event),

    // Requests
    /// Network request.
    #[from]
    NetworkRequest(NetworkRequest<NodeId, Message>),
    /// Deploy queue request.
    #[from]
    DeployQueueRequest(DeployQueueRequest),

    // Announcements
    /// Network announcement.
    #[from]
    NetworkAnnouncement(NetworkAnnouncement<NodeId, Message>),
    /// Storage announcement.
    #[from]
    StorageAnnouncement(StorageAnnouncement<Storage>),
    /// API server announcement.
    #[from]
    ApiServerAnnouncement(ApiServerAnnouncement),
}

impl From<ApiRequest> for Event {
    fn from(request: ApiRequest) -> Self {
        Event::ApiServer(api_server::Event::ApiRequest(request))
    }
}

impl From<NetworkRequest<NodeId, consensus::ConsensusMessage>> for Event {
    fn from(request: NetworkRequest<NodeId, consensus::ConsensusMessage>) -> Self {
        Event::NetworkRequest(request.map_payload(Message::from))
    }
}

impl From<NetworkRequest<NodeId, pinger::Message>> for Event {
    fn from(request: NetworkRequest<NodeId, pinger::Message>) -> Self {
        Event::NetworkRequest(request.map_payload(Message::from))
    }
}

impl From<NetworkRequest<NodeId, deploy_gossiper::Message>> for Event {
    fn from(request: NetworkRequest<NodeId, deploy_gossiper::Message>) -> Self {
        Event::NetworkRequest(request.map_payload(Message::from))
    }
}

impl From<ContractRuntimeRequest> for Event {
    fn from(request: ContractRuntimeRequest) -> Event {
        Event::ContractRuntime(contract_runtime::Event::Request(request))
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Network(event) => write!(f, "network: {}", event),
            Event::DeployQueue(event) => write!(f, "deploy queue: {}", event),
            Event::Pinger(event) => write!(f, "pinger: {}", event),
            Event::Storage(event) => write!(f, "storage: {}", event),
            Event::ApiServer(event) => write!(f, "api server: {}", event),
            Event::Consensus(event) => write!(f, "consensus: {}", event),
            Event::DeployGossiper(event) => write!(f, "deploy gossiper: {}", event),
            Event::ContractRuntime(event) => write!(f, "contract runtime: {}", event),
            Event::NetworkRequest(req) => write!(f, "network request: {}", req),
            Event::DeployQueueRequest(req) => write!(f, "deploy queue request: {}", req),
            Event::NetworkAnnouncement(ann) => write!(f, "network announcement: {}", ann),
            Event::ApiServerAnnouncement(ann) => write!(f, "api server announcement: {}", ann),
            Event::StorageAnnouncement(ann) => write!(f, "storage announcement: {}", ann),
        }
    }
}

/// Validator node reactor.
#[derive(Debug)]
pub struct Reactor {
    net: SmallNetwork<Event, Message>,
    pinger: Pinger,
    storage: Storage,
    contract_runtime: ContractRuntime,
    api_server: ApiServer,
    consensus: EraSupervisor<NodeId>,
    deploy_gossiper: DeployGossiper,
    deploy_buffer: DeployBuffer,
}

impl Reactor {
    fn init(
        config: Config,
        event_queue: EventQueueHandle<Event>,
        storage: Storage,
        contract_runtime: ContractRuntime,
    ) -> Result<(Self, Effects<Event>), Error> {
        let effect_builder = EffectBuilder::new(event_queue);
        let (net, net_effects) = SmallNetwork::new(event_queue, config.validator_net)?;

        let (pinger, pinger_effects) = Pinger::new(effect_builder);
        let api_server = ApiServer::new(config.http_server, effect_builder);
        let timestamp = Timestamp::now();
        let (consensus, consensus_effects) = EraSupervisor::new(timestamp, effect_builder);
        let deploy_gossiper = DeployGossiper::new(config.gossip);
        let deploy_buffer = DeployBuffer::new();

        let mut effects = reactor::wrap_effects(Event::Network, net_effects);
        effects.extend(reactor::wrap_effects(Event::Pinger, pinger_effects));
        effects.extend(reactor::wrap_effects(Event::Consensus, consensus_effects));

        Ok((
            Reactor {
                net,
                pinger,
                storage,
                api_server,
                consensus,
                deploy_gossiper,
                contract_runtime,
                deploy_buffer,
            },
            effects,
        ))
    }
}

impl reactor::Reactor for Reactor {
    type Event = Event;
    type Config = Config;
    type Error = Error;

    fn new<R: Rng + ?Sized>(
        config: Self::Config,
        event_queue: EventQueueHandle<Self::Event>,
        _rng: &mut R,
    ) -> Result<(Self, Effects<Event>), Error> {
        let storage = Storage::new(&config.storage)?;
        let contract_runtime = ContractRuntime::new(&config.storage, config.contract_runtime)?;
        Self::init(config, event_queue, storage, contract_runtime)
    }

    fn dispatch_event<R: Rng + ?Sized>(
        &mut self,
        effect_builder: EffectBuilder<Self::Event>,
        rng: &mut R,
        event: Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Network(event) => reactor::wrap_effects(
                Event::Network,
                self.net.handle_event(effect_builder, rng, event),
            ),
            Event::DeployQueue(event) => reactor::wrap_effects(
                Event::DeployQueue,
                self.deploy_buffer.handle_event(effect_builder, rng, event),
            ),
            Event::Pinger(event) => reactor::wrap_effects(
                Event::Pinger,
                self.pinger.handle_event(effect_builder, rng, event),
            ),
            Event::Storage(event) => reactor::wrap_effects(
                Event::Storage,
                self.storage.handle_event(effect_builder, rng, event),
            ),
            Event::ApiServer(event) => reactor::wrap_effects(
                Event::ApiServer,
                self.api_server.handle_event(effect_builder, rng, event),
            ),
            Event::Consensus(event) => reactor::wrap_effects(
                Event::Consensus,
                self.consensus.handle_event(effect_builder, rng, event),
            ),
            Event::DeployGossiper(event) => reactor::wrap_effects(
                Event::DeployGossiper,
                self.deploy_gossiper
                    .handle_event(effect_builder, rng, event),
            ),
            Event::ContractRuntime(event) => reactor::wrap_effects(
                Event::ContractRuntime,
                self.contract_runtime
                    .handle_event(effect_builder, rng, event),
            ),
            // Requests:
            Event::NetworkRequest(req) => self.dispatch_event(
                effect_builder,
                rng,
                Event::Network(small_network::Event::from(req)),
            ),
            Event::DeployQueueRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::DeployQueue(req.into()))
            }

            // Announcements:
            Event::NetworkAnnouncement(NetworkAnnouncement::MessageReceived {
                sender,
                payload,
            }) => {
                let reactor_event = match payload {
                    Message::Consensus(msg) => {
                        Event::Consensus(consensus::Event::MessageReceived { sender, msg })
                    }
                    Message::Pinger(msg) => {
                        Event::Pinger(pinger::Event::MessageReceived { sender, msg })
                    }
                    Message::DeployGossiper(message) => {
                        Event::DeployGossiper(deploy_gossiper::Event::MessageReceived {
                            sender,
                            message,
                        })
                    }
                };

                // Any incoming message is one for the pinger.
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::ApiServerAnnouncement(ApiServerAnnouncement::DeployReceived { deploy }) => {
                let event = deploy_gossiper::Event::DeployReceived { deploy };
                self.dispatch_event(effect_builder, rng, Event::DeployGossiper(event))
            }
            Event::StorageAnnouncement(StorageAnnouncement::StoredDeploy {
                deploy_hash,
                deploy_header,
            }) => {
                if self.deploy_buffer.add_deploy(deploy_hash, deploy_header) {
                    info!("Added deploy {} to the buffer.", deploy_hash);
                } else {
                    info!("Deploy {} rejected from the buffer.", deploy_hash);
                }
                Effects::new()
            }
        }
    }
}

impl reactor::ReactorExt<initializer::Reactor> for Reactor {
    fn new_from(
        event_queue: EventQueueHandle<Self::Event>,
        initializer_reactor: initializer::Reactor,
    ) -> Result<(Self, Effects<Self::Event>), Error> {
        let (config, storage, contract_runtime) = initializer_reactor.destructure();
        Self::init(config, event_queue, storage, contract_runtime)
    }
}
