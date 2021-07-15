//! Reactor for validator nodes.
//!
//! Validator nodes join the validator-only network upon startup.

mod config;
mod error;
mod memory_metrics;
#[cfg(test)]
mod tests;

use std::{
    env,
    fmt::{self, Debug, Display, Formatter},
    path::PathBuf,
    sync::Arc,
};

use datasize::DataSize;
use derive_more::From;
use prometheus::Registry;
use reactor::ReactorEvent;
use serde::Serialize;
use tracing::{debug, error, trace, warn};

#[cfg(test)]
use crate::testing::network::NetworkedReactor;

use crate::{
    components::{
        block_proposer::{self, BlockProposer},
        block_validator::{self, BlockValidator},
        chainspec_loader::{self, ChainspecLoader},
        consensus::{self, EraSupervisor, HighwayProtocol},
        contract_runtime::{self, ContractRuntime},
        deploy_acceptor::{self, DeployAcceptor},
        event_stream_server::{self, EventStreamServer},
        fetcher::{self, FetchedOrNotFound, Fetcher},
        gossiper::{self, Gossiper},
        linear_chain, linear_chain_sync,
        metrics::Metrics,
        network::{self, Network, NetworkIdentity, ENABLE_LIBP2P_NET_ENV_VAR},
        rest_server::{self, RestServer},
        rpc_server::{self, RpcServer},
        small_network::{self, GossipedAddress, SmallNetwork, SmallNetworkIdentity},
        storage::{self, Storage},
        Component,
    },
    effect::{
        announcements::{
            BlocklistAnnouncement, ChainspecLoaderAnnouncement, ConsensusAnnouncement,
            ContractRuntimeAnnouncement, ControlAnnouncement, DeployAcceptorAnnouncement,
            GossiperAnnouncement, LinearChainAnnouncement, LinearChainBlock, NetworkAnnouncement,
            RpcServerAnnouncement,
        },
        requests::{
            BlockProposerRequest, BlockValidationRequest, ChainspecLoaderRequest, ConsensusRequest,
            ContractRuntimeRequest, FetcherRequest, LinearChainRequest, MetricsRequest,
            NetworkInfoRequest, NetworkRequest, RestRequest, RpcRequest, StateStoreRequest,
            StorageRequest,
        },
        EffectBuilder, EffectExt, Effects,
    },
    protocol::Message,
    reactor::{self, event_queue_metrics::EventQueueMetrics, EventQueueHandle, ReactorExit},
    types::{BlockHash, BlockHeader, Deploy, DeployHash, ExitCode, NodeId, Tag},
    utils::{Source, WithDir},
    NodeRng,
};
use casper_execution_engine::shared::newtypes::Blake2bHash;
pub use config::Config;
pub use error::Error;
use linear_chain::LinearChainComponent;
use memory_metrics::MemoryMetrics;

/// Top-level event for the reactor.
#[derive(Debug, From, Serialize)]
#[must_use]
pub enum Event {
    /// Network event.
    #[from]
    Network(network::Event<Message>),
    /// Small network event.
    #[from]
    SmallNetwork(small_network::Event<Message>),
    /// Block proposer event.
    #[from]
    BlockProposer(#[serde(skip_serializing)] block_proposer::Event),
    #[from]
    /// Storage event.
    Storage(#[serde(skip_serializing)] storage::Event),
    #[from]
    /// RPC server event.
    RpcServer(#[serde(skip_serializing)] rpc_server::Event),
    #[from]
    /// REST server event.
    RestServer(#[serde(skip_serializing)] rest_server::Event),
    #[from]
    /// Event stream server event.
    EventStreamServer(#[serde(skip_serializing)] event_stream_server::Event),
    #[from]
    /// Chainspec Loader event.
    ChainspecLoader(#[serde(skip_serializing)] chainspec_loader::Event),
    #[from]
    /// Consensus event.
    Consensus(#[serde(skip_serializing)] consensus::Event<NodeId>),
    /// Deploy acceptor event.
    #[from]
    DeployAcceptor(#[serde(skip_serializing)] deploy_acceptor::Event),
    /// Deploy fetcher event.
    #[from]
    DeployFetcher(#[serde(skip_serializing)] fetcher::Event<Deploy>),
    /// Deploy gossiper event.
    #[from]
    DeployGossiper(#[serde(skip_serializing)] gossiper::Event<Deploy>),
    /// Address gossiper event.
    #[from]
    AddressGossiper(gossiper::Event<GossipedAddress>),
    /// Contract runtime event.
    #[from]
    ContractRuntime(#[serde(skip_serializing)] contract_runtime::Event),
    /// Block validator event.
    #[from]
    BlockValidator(#[serde(skip_serializing)] block_validator::Event<NodeId>),
    /// Linear chain event.
    #[from]
    LinearChain(#[serde(skip_serializing)] linear_chain::Event<NodeId>),

    // Requests
    /// Network request.
    #[from]
    NetworkRequest(#[serde(skip_serializing)] NetworkRequest<NodeId, Message>),
    /// Network info request.
    #[from]
    NetworkInfoRequest(#[serde(skip_serializing)] NetworkInfoRequest<NodeId>),
    /// Deploy fetcher request.
    #[from]
    DeployFetcherRequest(#[serde(skip_serializing)] FetcherRequest<NodeId, Deploy>),
    /// Block proposer request.
    #[from]
    BlockProposerRequest(#[serde(skip_serializing)] BlockProposerRequest),
    /// Block validator request.
    #[from]
    BlockValidatorRequest(#[serde(skip_serializing)] BlockValidationRequest<NodeId>),
    /// Metrics request.
    #[from]
    MetricsRequest(#[serde(skip_serializing)] MetricsRequest),
    /// Chainspec info request
    #[from]
    ChainspecLoaderRequest(#[serde(skip_serializing)] ChainspecLoaderRequest),
    /// Storage request.
    #[from]
    StorageRequest(#[serde(skip_serializing)] StorageRequest),
    /// Request for state storage.
    #[from]
    StateStoreRequest(StateStoreRequest),

    // Announcements
    /// Control announcement.
    #[from]
    ControlAnnouncement(ControlAnnouncement),
    /// Network announcement.
    #[from]
    NetworkAnnouncement(#[serde(skip_serializing)] NetworkAnnouncement<NodeId, Message>),
    /// API server announcement.
    #[from]
    RpcServerAnnouncement(#[serde(skip_serializing)] RpcServerAnnouncement),
    /// DeployAcceptor announcement.
    #[from]
    DeployAcceptorAnnouncement(#[serde(skip_serializing)] DeployAcceptorAnnouncement<NodeId>),
    /// Consensus announcement.
    #[from]
    ConsensusAnnouncement(#[serde(skip_serializing)] ConsensusAnnouncement),
    /// ContractRuntime announcement.
    #[from]
    ContractRuntimeAnnouncement(#[serde(skip_serializing)] ContractRuntimeAnnouncement),
    /// Deploy Gossiper announcement.
    #[from]
    DeployGossiperAnnouncement(#[serde(skip_serializing)] GossiperAnnouncement<Deploy>),
    /// Address Gossiper announcement.
    #[from]
    AddressGossiperAnnouncement(#[serde(skip_serializing)] GossiperAnnouncement<GossipedAddress>),
    /// Linear chain announcement.
    #[from]
    LinearChainAnnouncement(#[serde(skip_serializing)] LinearChainAnnouncement),
    /// Chainspec loader announcement.
    #[from]
    ChainspecLoaderAnnouncement(#[serde(skip_serializing)] ChainspecLoaderAnnouncement),
    /// Blocklist announcement.
    #[from]
    BlocklistAnnouncement(BlocklistAnnouncement<NodeId>),
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

impl From<RpcRequest<NodeId>> for Event {
    fn from(request: RpcRequest<NodeId>) -> Self {
        Event::RpcServer(rpc_server::Event::RpcRequest(request))
    }
}

impl From<RestRequest<NodeId>> for Event {
    fn from(request: RestRequest<NodeId>) -> Self {
        Event::RestServer(rest_server::Event::RestRequest(request))
    }
}

impl From<NetworkRequest<NodeId, consensus::ConsensusMessage>> for Event {
    fn from(request: NetworkRequest<NodeId, consensus::ConsensusMessage>) -> Self {
        Event::NetworkRequest(request.map_payload(Message::from))
    }
}

impl From<NetworkRequest<NodeId, gossiper::Message<Deploy>>> for Event {
    fn from(request: NetworkRequest<NodeId, gossiper::Message<Deploy>>) -> Self {
        Event::NetworkRequest(request.map_payload(Message::from))
    }
}

impl From<NetworkRequest<NodeId, gossiper::Message<GossipedAddress>>> for Event {
    fn from(request: NetworkRequest<NodeId, gossiper::Message<GossipedAddress>>) -> Self {
        Event::NetworkRequest(request.map_payload(Message::from))
    }
}

impl From<ContractRuntimeRequest> for Event {
    fn from(request: ContractRuntimeRequest) -> Event {
        Event::ContractRuntime(contract_runtime::Event::Request(Box::new(request)))
    }
}

impl From<ConsensusRequest> for Event {
    fn from(request: ConsensusRequest) -> Self {
        Event::Consensus(consensus::Event::ConsensusRequest(request))
    }
}

impl From<LinearChainRequest<NodeId>> for Event {
    fn from(request: LinearChainRequest<NodeId>) -> Self {
        Event::LinearChain(linear_chain::Event::Request(request))
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Network(event) => write!(f, "network: {}", event),
            Event::SmallNetwork(event) => write!(f, "small network: {}", event),
            Event::BlockProposer(event) => write!(f, "block proposer: {}", event),
            Event::Storage(event) => write!(f, "storage: {}", event),
            Event::RpcServer(event) => write!(f, "rpc server: {}", event),
            Event::RestServer(event) => write!(f, "rest server: {}", event),
            Event::EventStreamServer(event) => write!(f, "event stream server: {}", event),
            Event::ChainspecLoader(event) => write!(f, "chainspec loader: {}", event),
            Event::Consensus(event) => write!(f, "consensus: {}", event),
            Event::DeployAcceptor(event) => write!(f, "deploy acceptor: {}", event),
            Event::DeployFetcher(event) => write!(f, "deploy fetcher: {}", event),
            Event::DeployGossiper(event) => write!(f, "deploy gossiper: {}", event),
            Event::AddressGossiper(event) => write!(f, "address gossiper: {}", event),
            Event::ContractRuntime(event) => write!(f, "contract runtime: {:?}", event),
            Event::LinearChain(event) => write!(f, "linear-chain event {}", event),
            Event::BlockValidator(event) => write!(f, "block validator: {}", event),
            Event::NetworkRequest(req) => write!(f, "network request: {}", req),
            Event::NetworkInfoRequest(req) => write!(f, "network info request: {}", req),
            Event::ChainspecLoaderRequest(req) => write!(f, "chainspec loader request: {}", req),
            Event::StorageRequest(req) => write!(f, "storage request: {}", req),
            Event::StateStoreRequest(req) => write!(f, "state store request: {}", req),
            Event::DeployFetcherRequest(req) => write!(f, "deploy fetcher request: {}", req),
            Event::BlockProposerRequest(req) => write!(f, "block proposer request: {}", req),
            Event::BlockValidatorRequest(req) => {
                write!(f, "block validator request: {}", req)
            }
            Event::MetricsRequest(req) => write!(f, "metrics request: {}", req),
            Event::ControlAnnouncement(ctrl_ann) => write!(f, "control: {}", ctrl_ann),
            Event::NetworkAnnouncement(ann) => write!(f, "network announcement: {}", ann),
            Event::RpcServerAnnouncement(ann) => write!(f, "api server announcement: {}", ann),
            Event::DeployAcceptorAnnouncement(ann) => {
                write!(f, "deploy acceptor announcement: {}", ann)
            }
            Event::ConsensusAnnouncement(ann) => write!(f, "consensus announcement: {}", ann),
            Event::ContractRuntimeAnnouncement(ann) => {
                write!(f, "block-executor announcement: {}", ann)
            }
            Event::DeployGossiperAnnouncement(ann) => {
                write!(f, "deploy gossiper announcement: {}", ann)
            }
            Event::AddressGossiperAnnouncement(ann) => {
                write!(f, "address gossiper announcement: {}", ann)
            }
            Event::LinearChainAnnouncement(ann) => write!(f, "linear chain announcement: {}", ann),
            Event::ChainspecLoaderAnnouncement(ann) => {
                write!(f, "chainspec loader announcement: {}", ann)
            }
            Event::BlocklistAnnouncement(ann) => {
                write!(f, "blocklist announcement: {}", ann)
            }
        }
    }
}

/// The configuration needed to initialize a Validator reactor
pub struct ValidatorInitConfig {
    pub(super) root: PathBuf,
    pub(super) config: Config,
    pub(super) chainspec_loader: ChainspecLoader,
    pub(super) storage: Storage,
    pub(super) contract_runtime: ContractRuntime,
    pub(super) maybe_latest_block_header: Option<BlockHeader>,
    pub(super) event_stream_server: EventStreamServer,
    pub(super) small_network_identity: SmallNetworkIdentity,
    pub(super) network_identity: NetworkIdentity,
}

#[cfg(test)]
impl ValidatorInitConfig {
    /// Inspect storage.
    pub(crate) fn storage(&self) -> &Storage {
        &self.storage
    }
}

impl Debug for ValidatorInitConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ValidatorInitConfig {{ .. }}")
    }
}

/// Validator node reactor.
#[derive(DataSize, Debug)]
pub struct Reactor {
    metrics: Metrics,
    small_network: SmallNetwork<Event, Message>,
    network: Network<Event, Message>,
    address_gossiper: Gossiper<GossipedAddress, Event>,
    storage: Storage,
    contract_runtime: ContractRuntime,
    rpc_server: RpcServer,
    rest_server: RestServer,
    event_stream_server: EventStreamServer,
    chainspec_loader: ChainspecLoader,
    consensus: EraSupervisor<NodeId>,
    #[data_size(skip)]
    deploy_acceptor: DeployAcceptor,
    deploy_fetcher: Fetcher<Deploy>,
    deploy_gossiper: Gossiper<Deploy, Event>,
    block_proposer: BlockProposer,
    block_validator: BlockValidator<NodeId>,
    linear_chain: LinearChainComponent<NodeId>,

    // Non-components.
    #[data_size(skip)] // Never allocates heap data.
    memory_metrics: MemoryMetrics,

    #[data_size(skip)]
    event_queue_metrics: EventQueueMetrics,
}

#[cfg(test)]
impl Reactor {
    /// Inspect consensus.
    pub(crate) fn consensus(&self) -> &EraSupervisor<NodeId> {
        &self.consensus
    }
    /// Inspect storage.
    pub(crate) fn storage(&self) -> &Storage {
        &self.storage
    }
}

impl reactor::Reactor for Reactor {
    type Event = Event;

    // The "configuration" is in fact the whole state of the joiner reactor, which we
    // deconstruct and reuse.
    type Config = ValidatorInitConfig;
    type Error = Error;

    fn new(
        config: Self::Config,
        registry: &Registry,
        event_queue: EventQueueHandle<Self::Event>,
        _rng: &mut NodeRng,
    ) -> Result<(Self, Effects<Event>), Error> {
        let ValidatorInitConfig {
            root,
            config,
            chainspec_loader,
            storage,
            mut contract_runtime,
            maybe_latest_block_header,
            event_stream_server,
            small_network_identity,
            network_identity,
        } = config;

        let memory_metrics = MemoryMetrics::new(registry.clone())?;

        let event_queue_metrics = EventQueueMetrics::new(registry.clone(), event_queue)?;

        let metrics = Metrics::new(registry.clone());

        let effect_builder = EffectBuilder::new(event_queue);
        let network_config = network::Config::from(&config.network);
        let (network, network_effects) = Network::new(
            event_queue,
            network_config,
            registry,
            network_identity,
            chainspec_loader.chainspec(),
        )?;

        let address_gossiper =
            Gossiper::new_for_complete_items("address_gossiper", config.gossip, registry)?;

        let protocol_version = &chainspec_loader.chainspec().protocol_config.version;
        let rpc_server =
            RpcServer::new(config.rpc_server.clone(), effect_builder, *protocol_version)?;
        let rest_server = RestServer::new(
            config.rest_server.clone(),
            effect_builder,
            *protocol_version,
        )?;

        let deploy_acceptor =
            DeployAcceptor::new(config.deploy_acceptor, &*chainspec_loader.chainspec());
        let deploy_fetcher = Fetcher::new("deploy", config.fetcher, &registry)?;
        let deploy_gossiper = Gossiper::new_for_partial_items(
            "deploy_gossiper",
            config.gossip,
            gossiper::get_deploy_from_storage::<Deploy, Event>,
            registry,
        )?;
        let (block_proposer, block_proposer_effects) = BlockProposer::new(
            registry.clone(),
            effect_builder,
            maybe_latest_block_header
                .as_ref()
                .map(|block_header| block_header.height() + 1)
                .unwrap_or(0),
            chainspec_loader.chainspec().as_ref(),
        )?;

        let initial_era = maybe_latest_block_header.as_ref().map_or_else(
            || chainspec_loader.initial_era(),
            |block_header| block_header.next_block_era_id(),
        );

        let (small_network, small_network_effects) = SmallNetwork::new(
            event_queue,
            config.network,
            Some(WithDir::new(&root, &config.consensus)),
            registry,
            small_network_identity,
            chainspec_loader.chainspec().as_ref(),
            Some(initial_era),
        )?;

        let mut effects = reactor::wrap_effects(Event::BlockProposer, block_proposer_effects);

        let maybe_next_activation_point = chainspec_loader
            .next_upgrade()
            .map(|next_upgrade| next_upgrade.activation_point());
        let (consensus, init_consensus_effects) = EraSupervisor::new(
            initial_era,
            WithDir::new(root, config.consensus),
            effect_builder,
            chainspec_loader.chainspec().as_ref().into(),
            maybe_latest_block_header.as_ref(),
            maybe_next_activation_point,
            registry,
            Box::new(HighwayProtocol::new_boxed),
        )?;
        effects.extend(reactor::wrap_effects(
            Event::Consensus,
            init_consensus_effects,
        ));
        contract_runtime.set_initial_state(
            chainspec_loader.initial_state_root_hash(),
            chainspec_loader.initial_block_header(),
        );
        contract_runtime.set_parent_map_from_block(maybe_latest_block_header);

        let block_validator = BlockValidator::new(Arc::clone(&chainspec_loader.chainspec()));
        let linear_chain = linear_chain::LinearChainComponent::new(
            &registry,
            *protocol_version,
            chainspec_loader.chainspec().clone(),
        )?;

        effects.extend(reactor::wrap_effects(Event::Network, network_effects));
        effects.extend(reactor::wrap_effects(
            Event::SmallNetwork,
            small_network_effects,
        ));
        effects.extend(reactor::wrap_effects(
            Event::ChainspecLoader,
            chainspec_loader.start_checking_for_upgrades(effect_builder),
        ));

        Ok((
            Reactor {
                metrics,
                network,
                small_network,
                address_gossiper,
                storage,
                contract_runtime,
                rpc_server,
                rest_server,
                event_stream_server,
                chainspec_loader,
                consensus,
                deploy_acceptor,
                deploy_fetcher,
                deploy_gossiper,
                block_proposer,
                block_validator,
                linear_chain,
                memory_metrics,
                event_queue_metrics,
            },
            effects,
        ))
    }

    fn dispatch_event(
        &mut self,
        effect_builder: EffectBuilder<Self::Event>,
        rng: &mut NodeRng,
        event: Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Network(event) => reactor::wrap_effects(
                Event::Network,
                self.network.handle_event(effect_builder, rng, event),
            ),
            Event::SmallNetwork(event) => reactor::wrap_effects(
                Event::SmallNetwork,
                self.small_network.handle_event(effect_builder, rng, event),
            ),
            Event::BlockProposer(event) => reactor::wrap_effects(
                Event::BlockProposer,
                self.block_proposer.handle_event(effect_builder, rng, event),
            ),
            Event::Storage(event) => reactor::wrap_effects(
                Event::Storage,
                self.storage.handle_event(effect_builder, rng, event),
            ),
            Event::RpcServer(event) => reactor::wrap_effects(
                Event::RpcServer,
                self.rpc_server.handle_event(effect_builder, rng, event),
            ),
            Event::RestServer(event) => reactor::wrap_effects(
                Event::RestServer,
                self.rest_server.handle_event(effect_builder, rng, event),
            ),
            Event::EventStreamServer(event) => reactor::wrap_effects(
                Event::EventStreamServer,
                self.event_stream_server
                    .handle_event(effect_builder, rng, event),
            ),
            Event::ChainspecLoader(event) => reactor::wrap_effects(
                Event::ChainspecLoader,
                self.chainspec_loader
                    .handle_event(effect_builder, rng, event),
            ),
            Event::Consensus(event) => reactor::wrap_effects(
                Event::Consensus,
                self.consensus.handle_event(effect_builder, rng, event),
            ),
            Event::DeployAcceptor(event) => reactor::wrap_effects(
                Event::DeployAcceptor,
                self.deploy_acceptor
                    .handle_event(effect_builder, rng, event),
            ),
            Event::DeployFetcher(event) => reactor::wrap_effects(
                Event::DeployFetcher,
                self.deploy_fetcher.handle_event(effect_builder, rng, event),
            ),
            Event::DeployGossiper(event) => reactor::wrap_effects(
                Event::DeployGossiper,
                self.deploy_gossiper
                    .handle_event(effect_builder, rng, event),
            ),
            Event::AddressGossiper(event) => reactor::wrap_effects(
                Event::AddressGossiper,
                self.address_gossiper
                    .handle_event(effect_builder, rng, event),
            ),
            Event::ContractRuntime(event) => reactor::wrap_effects(
                Event::ContractRuntime,
                self.contract_runtime
                    .handle_event(effect_builder, rng, event),
            ),
            Event::BlockValidator(event) => reactor::wrap_effects(
                Event::BlockValidator,
                self.block_validator
                    .handle_event(effect_builder, rng, event),
            ),
            Event::LinearChain(event) => reactor::wrap_effects(
                Event::LinearChain,
                self.linear_chain.handle_event(effect_builder, rng, event),
            ),

            // Requests:
            Event::NetworkRequest(req) => {
                let event = if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_ok() {
                    Event::Network(network::Event::from(req))
                } else {
                    Event::SmallNetwork(small_network::Event::from(req))
                };
                self.dispatch_event(effect_builder, rng, event)
            }
            Event::NetworkInfoRequest(req) => {
                let event = if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_ok() {
                    Event::Network(network::Event::from(req))
                } else {
                    Event::SmallNetwork(small_network::Event::from(req))
                };
                self.dispatch_event(effect_builder, rng, event)
            }
            Event::DeployFetcherRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::DeployFetcher(req.into()))
            }
            Event::BlockProposerRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::BlockProposer(req.into()))
            }
            Event::BlockValidatorRequest(req) => self.dispatch_event(
                effect_builder,
                rng,
                Event::BlockValidator(block_validator::Event::from(req)),
            ),
            Event::MetricsRequest(req) => reactor::wrap_effects(
                Event::MetricsRequest,
                self.metrics.handle_event(effect_builder, rng, req),
            ),
            Event::ChainspecLoaderRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::ChainspecLoader(req.into()))
            }
            Event::StorageRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::Storage(req.into()))
            }
            Event::StateStoreRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::Storage(req.into()))
            }

            // Announcements:
            Event::ControlAnnouncement(ctrl_ann) => {
                unreachable!("unhandled control announcement: {}", ctrl_ann)
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::MessageReceived {
                sender,
                payload,
            }) => {
                let reactor_event = match payload {
                    Message::Consensus(msg) => {
                        Event::Consensus(consensus::Event::MessageReceived { sender, msg })
                    }
                    Message::DeployGossiper(message) => {
                        Event::DeployGossiper(gossiper::Event::MessageReceived { sender, message })
                    }
                    Message::AddressGossiper(message) => {
                        Event::AddressGossiper(gossiper::Event::MessageReceived { sender, message })
                    }
                    Message::GetRequest { tag, serialized_id } => match tag {
                        Tag::Deploy => {
                            let deploy_hash: DeployHash = match bincode::deserialize(&serialized_id)
                            {
                                Ok(hash) => hash,
                                Err(error) => {
                                    error!(
                                        "failed to decode {:?} from {}: {}",
                                        serialized_id, sender, error
                                    );
                                    return Effects::new();
                                }
                            };

                            let fetched_or_not_found_deploy = match self
                                .storage
                                .handle_legacy_direct_deploy_request(deploy_hash)
                            {
                                Some(deploy) => FetchedOrNotFound::Fetched(deploy),
                                None => FetchedOrNotFound::NotFound(deploy_hash),
                            };
                            match Message::new_get_response(&fetched_or_not_found_deploy) {
                                Ok(message) => {
                                    return effect_builder.send_message(sender, message).ignore();
                                }
                                Err(error) => {
                                    error!("failed to create get-response: {}", error);
                                    return Effects::new();
                                }
                            };
                        }
                        Tag::Block => {
                            let block_hash = match bincode::deserialize(&serialized_id) {
                                Ok(hash) => hash,
                                Err(error) => {
                                    error!(
                                        "failed to decode {:?} from {}: {}",
                                        serialized_id, sender, error
                                    );
                                    return Effects::new();
                                }
                            };
                            Event::LinearChain(linear_chain::Event::Request(
                                LinearChainRequest::BlockRequest(block_hash, sender),
                            ))
                        }
                        Tag::BlockAndMetadataByHeight => {
                            let height = match bincode::deserialize(&serialized_id) {
                                Ok(block_by_height) => block_by_height,
                                Err(error) => {
                                    error!(
                                        "failed to decode {:?} from {}: {}",
                                        serialized_id, sender, error
                                    );
                                    return Effects::new();
                                }
                            };
                            Event::LinearChain(linear_chain::Event::Request(
                                LinearChainRequest::BlockWithMetadataAtHeight(height, sender),
                            ))
                        }
                        Tag::GossipedAddress => {
                            warn!("received get request for gossiped-address from {}", sender);
                            return Effects::new();
                        }
                        Tag::BlockHeaderByHash => {
                            let block_hash: BlockHash = match bincode::deserialize(&serialized_id) {
                                Ok(block_hash) => block_hash,
                                Err(error) => {
                                    error!(
                                        "failed to decode {:?} from {}: {}",
                                        serialized_id, sender, error
                                    );
                                    return Effects::new();
                                }
                            };

                            let fetched_or_not_found_block_header = match self
                                .storage
                                .read_block_header_by_hash(&block_hash)
                            {
                                Ok(Some(block_header)) => FetchedOrNotFound::Fetched(block_header),
                                Ok(None) => FetchedOrNotFound::NotFound(block_hash),
                                Err(error) => {
                                    error!(
                                        "failed to get {} for {}: {}",
                                        block_hash, sender, error
                                    );
                                    FetchedOrNotFound::NotFound(block_hash)
                                }
                            };

                            match Message::new_get_response(&fetched_or_not_found_block_header) {
                                Err(error) => {
                                    error!("failed to create get-response: {}", error);
                                    return Effects::new();
                                }
                                Ok(message) => {
                                    return effect_builder.send_message(sender, message).ignore();
                                }
                            };
                        }
                        Tag::BlockHeaderAndFinalitySignaturesByHeight => {
                            let block_height: u64 = match bincode::deserialize(&serialized_id) {
                                Ok(block_height) => block_height,
                                Err(error) => {
                                    error!(
                                        "failed to decode {:?} from {}: {}",
                                        serialized_id, sender, error
                                    );
                                    return Effects::new();
                                }
                            };
                            let maybe_header_with_sigs = self
                                .storage
                                .read_block_header_and_finality_signatures_by_height(block_height)
                                .unwrap_or_else(|error| {
                                    error!(
                                        "failed to get {} for {}: {}",
                                        block_height, sender, error
                                    );
                                    None
                                });
                            let chainspec = Arc::clone(self.chainspec_loader.chainspec());

                            return async move {
                                let mut fully_signed = false;
                                if let Some(header_with_sigs) = &maybe_header_with_sigs {
                                    if let Some(validator_weights) = effect_builder
                                        .get_era_validators(header_with_sigs.block_header.era_id())
                                        .await
                                    {
                                        fully_signed =
                                            linear_chain_sync::weigh_finality_signatures(
                                                &validator_weights,
                                                chainspec
                                                    .highway_config
                                                    .finality_threshold_fraction,
                                                &header_with_sigs.block_signatures,
                                            )
                                            .is_ok();
                                    }
                                }
                                let fetch_or_not_found_header_with_sigs =
                                    match maybe_header_with_sigs.filter(|_| fully_signed) {
                                        None => FetchedOrNotFound::NotFound(block_height),
                                        Some(header) => FetchedOrNotFound::Fetched(header),
                                    };
                                match Message::new_get_response(
                                    &fetch_or_not_found_header_with_sigs,
                                ) {
                                    Ok(message) => {
                                        effect_builder.send_message(sender, message).await
                                    }
                                    Err(error) => error!("failed to create get-response {}", error),
                                }
                            }
                            .ignore();
                        }
                        Tag::Trie => {
                            let trie_key: Blake2bHash = match bincode::deserialize(&serialized_id) {
                                Ok(trie_key) => trie_key,
                                Err(error) => {
                                    error!(
                                        "failed to decode {:?} from {}: {}",
                                        serialized_id, sender, error
                                    );
                                    return Effects::new();
                                }
                            };
                            let trie = match self.contract_runtime.read_trie(trie_key) {
                                Ok(Some(trie)) => FetchedOrNotFound::Fetched(trie),
                                Ok(None) => {
                                    debug!(
                                        "failed to get trie with trie_key {} for {}",
                                        trie_key, sender
                                    );
                                    FetchedOrNotFound::NotFound(trie_key)
                                }
                                Err(error) => {
                                    error!(
                                        "error when trying to get trie with trie_key {} for {}: {}",
                                        trie_key, sender, error
                                    );
                                    FetchedOrNotFound::NotFound(trie_key)
                                }
                            };
                            match Message::new_get_response(&trie) {
                                Ok(message) => {
                                    return effect_builder.send_message(sender, message).ignore();
                                }
                                Err(error) => {
                                    error!(
                                        "failed to create get-response after retrieving \
                                            trie_key {}: {}",
                                        trie_key, error
                                    );
                                    return Effects::new();
                                }
                            };
                        }
                    },
                    Message::GetResponse {
                        tag,
                        serialized_item,
                    } => match tag {
                        Tag::Deploy => {
                            let deploy: Box<Deploy> = match bincode::deserialize::<
                                FetchedOrNotFound<Deploy, DeployHash>,
                            >(
                                &serialized_item
                            ) {
                                Ok(FetchedOrNotFound::Fetched(deploy)) => Box::new(deploy),
                                Ok(FetchedOrNotFound::NotFound(deploy_hash)) => {
                                    error!(
                                        "peer did not have deploy with hash {}: {}",
                                        sender, deploy_hash
                                    );
                                    return Effects::new();
                                }
                                Err(error) => {
                                    error!("failed to decode deploy from {}: {}", sender, error);
                                    return Effects::new();
                                }
                            };
                            Event::DeployAcceptor(deploy_acceptor::Event::Accept {
                                deploy,
                                source: Source::Peer(sender),
                                responder: None,
                            })
                        }
                        Tag::Block => {
                            error!(
                                "cannot handle get response for block-by-hash from {}",
                                sender
                            );
                            return Effects::new();
                        }
                        Tag::BlockAndMetadataByHeight => {
                            error!(
                                "cannot handle get response for block-by-height from {}",
                                sender
                            );
                            return Effects::new();
                        }
                        Tag::GossipedAddress => {
                            error!(
                                "cannot handle get response for gossiped-address from {}",
                                sender
                            );
                            return Effects::new();
                        }
                        Tag::BlockHeaderByHash => {
                            error!(
                                "cannot handle get response for block-header-by-hash from {}",
                                sender
                            );
                            return Effects::new();
                        }
                        Tag::BlockHeaderAndFinalitySignaturesByHeight => {
                            error!(
                                "cannot handle get response for \
                                 block-header-and-finality-signatures-by-height from {}",
                                sender
                            );
                            return Effects::new();
                        }
                        Tag::Trie => {
                            error!("cannot handle get response for read-trie from {}", sender);
                            return Effects::new();
                        }
                    },
                    Message::FinalitySignature(fs) => {
                        Event::LinearChain(linear_chain::Event::FinalitySignatureReceived(fs, true))
                    }
                };
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::GossipOurAddress(gossiped_address)) => {
                let event = gossiper::Event::ItemReceived {
                    item_id: gossiped_address,
                    source: Source::<NodeId>::Ourself,
                };
                self.dispatch_event(effect_builder, rng, Event::AddressGossiper(event))
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::NewPeer(_peer_id)) => {
                trace!("new peer announcement not handled in the validator reactor");
                Effects::new()
            }
            Event::RpcServerAnnouncement(RpcServerAnnouncement::DeployReceived {
                deploy,
                responder,
            }) => {
                let event = deploy_acceptor::Event::Accept {
                    deploy,
                    source: Source::<NodeId>::Client,
                    responder,
                };
                self.dispatch_event(effect_builder, rng, Event::DeployAcceptor(event))
            }
            Event::DeployAcceptorAnnouncement(DeployAcceptorAnnouncement::AcceptedNewDeploy {
                deploy,
                source,
            }) => {
                let event = gossiper::Event::ItemReceived {
                    item_id: *deploy.id(),
                    source: source.clone(),
                };
                let mut effects =
                    self.dispatch_event(effect_builder, rng, Event::DeployGossiper(event));

                let event = fetcher::Event::GotRemotely {
                    item: deploy,
                    source,
                };
                effects.extend(self.dispatch_event(
                    effect_builder,
                    rng,
                    Event::DeployFetcher(event),
                ));

                effects
            }
            Event::DeployAcceptorAnnouncement(DeployAcceptorAnnouncement::InvalidDeploy {
                deploy: _,
                source: _,
            }) => Effects::new(),
            Event::ConsensusAnnouncement(consensus_announcement) => match consensus_announcement {
                ConsensusAnnouncement::Finalized(block) => {
                    let reactor_event =
                        Event::BlockProposer(block_proposer::Event::FinalizedBlock(block));
                    self.dispatch_event(effect_builder, rng, reactor_event)
                }
                ConsensusAnnouncement::CreatedFinalitySignature(fs) => self.dispatch_event(
                    effect_builder,
                    rng,
                    Event::LinearChain(linear_chain::Event::FinalitySignatureReceived(fs, false)),
                ),
                ConsensusAnnouncement::Fault {
                    era_id,
                    public_key,
                    timestamp,
                } => {
                    let reactor_event =
                        Event::EventStreamServer(event_stream_server::Event::Fault {
                            era_id,
                            public_key: *public_key,
                            timestamp,
                        });
                    self.dispatch_event(effect_builder, rng, reactor_event)
                }
            },
            Event::ContractRuntimeAnnouncement(ContractRuntimeAnnouncement::LinearChainBlock(
                linear_chain_block,
            )) => {
                let LinearChainBlock {
                    block,
                    execution_results,
                } = *linear_chain_block;
                let mut effects = Effects::new();
                let block_hash = *block.hash();

                // send to linear chain
                let reactor_event = Event::LinearChain(linear_chain::Event::NewLinearChainBlock {
                    block: Box::new(block),
                    execution_results: execution_results
                        .iter()
                        .map(|(hash, (_header, results))| (*hash, results.clone()))
                        .collect(),
                });
                effects.extend(self.dispatch_event(effect_builder, rng, reactor_event));

                // send to event stream
                for (deploy_hash, (deploy_header, execution_result)) in execution_results {
                    let reactor_event =
                        Event::EventStreamServer(event_stream_server::Event::DeployProcessed {
                            deploy_hash,
                            deploy_header: Box::new(deploy_header),
                            block_hash,
                            execution_result: Box::new(execution_result),
                        });
                    effects.extend(self.dispatch_event(effect_builder, rng, reactor_event));
                }

                effects
            }
            Event::ContractRuntimeAnnouncement(ContractRuntimeAnnouncement::StepSuccess {
                era_id,
                execution_effect,
            }) => {
                let reactor_event = Event::EventStreamServer(event_stream_server::Event::Step {
                    era_id,
                    effect: execution_effect,
                });
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::DeployGossiperAnnouncement(GossiperAnnouncement::NewCompleteItem(
                gossiped_deploy_id,
            )) => {
                error!(%gossiped_deploy_id, "gossiper should not announce new deploy");
                Effects::new()
            }
            Event::DeployGossiperAnnouncement(GossiperAnnouncement::FinishedGossiping(
                gossiped_deploy_id,
            )) => {
                let reactor_event =
                    Event::BlockProposer(block_proposer::Event::BufferDeploy(gossiped_deploy_id));
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::AddressGossiperAnnouncement(GossiperAnnouncement::NewCompleteItem(
                gossiped_address,
            )) => {
                let reactor_event = Event::SmallNetwork(small_network::Event::PeerAddressReceived(
                    gossiped_address,
                ));
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::AddressGossiperAnnouncement(GossiperAnnouncement::FinishedGossiping(_)) => {
                // We don't care about completion of gossiping an address.
                Effects::new()
            }
            Event::LinearChainAnnouncement(LinearChainAnnouncement::BlockAdded(block)) => {
                let reactor_event_consensus = Event::Consensus(consensus::Event::BlockAdded(
                    Box::new(block.header().clone()),
                ));
                let reactor_event_es =
                    Event::EventStreamServer(event_stream_server::Event::BlockAdded(block.clone()));
                let mut effects = self.dispatch_event(effect_builder, rng, reactor_event_es);
                effects.extend(self.dispatch_event(effect_builder, rng, reactor_event_consensus));
                effects.extend(self.dispatch_event(
                    effect_builder,
                    rng,
                    small_network::Event::from(LinearChainAnnouncement::BlockAdded(block)).into(),
                ));
                effects
            }
            Event::LinearChainAnnouncement(LinearChainAnnouncement::NewFinalitySignature(fs)) => {
                let reactor_event =
                    Event::EventStreamServer(event_stream_server::Event::FinalitySignature(fs));
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::ChainspecLoaderAnnouncement(
                ChainspecLoaderAnnouncement::UpgradeActivationPointRead(next_upgrade),
            ) => {
                let reactor_event = Event::ChainspecLoader(
                    chainspec_loader::Event::GotNextUpgrade(next_upgrade.clone()),
                );
                let mut effects = self.dispatch_event(effect_builder, rng, reactor_event);

                let reactor_event = Event::Consensus(consensus::Event::GotUpgradeActivationPoint(
                    next_upgrade.activation_point(),
                ));
                effects.extend(self.dispatch_event(effect_builder, rng, reactor_event));
                effects
            }
            Event::BlocklistAnnouncement(ann) => {
                self.dispatch_event(effect_builder, rng, Event::SmallNetwork(ann.into()))
            }
        }
    }

    fn update_metrics(&mut self, event_queue_handle: EventQueueHandle<Self::Event>) {
        self.memory_metrics.estimate(&self);
        self.event_queue_metrics
            .record_event_queue_counts(&event_queue_handle)
    }

    fn maybe_exit(&self) -> Option<ReactorExit> {
        self.consensus
            .stop_for_upgrade()
            .then(|| ReactorExit::ProcessShouldExit(ExitCode::Success))
    }
}

#[cfg(test)]
impl NetworkedReactor for Reactor {
    type NodeId = NodeId;
    fn node_id(&self) -> Self::NodeId {
        if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_ok() {
            self.network.node_id()
        } else {
            self.small_network.node_id()
        }
    }
}
