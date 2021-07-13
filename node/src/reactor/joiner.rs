//! Reactor used to join the network.

mod memory_metrics;

use std::{
    collections::BTreeMap,
    env,
    fmt::{self, Display, Formatter},
    path::PathBuf,
    sync::Arc,
};

use datasize::DataSize;
use derive_more::From;
use memory_metrics::MemoryMetrics;
use prometheus::Registry;
use reactor::ReactorEvent;
use serde::Serialize;
use tracing::{debug, error, info, warn};

#[cfg(test)]
use crate::testing::network::NetworkedReactor;
use crate::{
    components::{
        block_validator::{self, BlockValidator},
        chainspec_loader::{self, ChainspecLoader},
        contract_runtime::ContractRuntime,
        deploy_acceptor::{self, DeployAcceptor},
        event_stream_server,
        event_stream_server::{DeployGetter, EventStreamServer},
        fetcher::{self, Fetcher},
        gossiper::{self, Gossiper},
        linear_chain,
        linear_chain_sync::{self, LinearChainSync},
        metrics::Metrics,
        network::{self, Network, NetworkIdentity, ENABLE_LIBP2P_NET_ENV_VAR},
        rest_server::{self, RestServer},
        small_network::{self, GossipedAddress, SmallNetwork, SmallNetworkIdentity},
        storage::{self, Storage},
        Component,
    },
    effect::{
        announcements::{
            ChainspecLoaderAnnouncement, ContractRuntimeAnnouncement, ControlAnnouncement,
            DeployAcceptorAnnouncement, GossiperAnnouncement, LinearChainAnnouncement,
            LinearChainBlock, NetworkAnnouncement,
        },
        requests::{
            BlockProposerRequest, BlockValidationRequest, ChainspecLoaderRequest, ConsensusRequest,
            ContractRuntimeRequest, FetcherRequest, LinearChainRequest, MetricsRequest,
            NetworkInfoRequest, NetworkRequest, RestRequest, StateStoreRequest, StorageRequest,
        },
        EffectBuilder, EffectExt, Effects,
    },
    protocol::Message,
    reactor::{
        self,
        event_queue_metrics::EventQueueMetrics,
        initializer,
        participating::{self, Error, ParticipatingInitConfig},
        EventQueueHandle, Finalize, ReactorExit,
    },
    types::{
        Block, BlockByHeight, BlockHeader, BlockHeaderWithMetadata, Deploy, ExitCode, NodeId, Tag,
        Timestamp,
    },
    utils::{Source, WithDir},
    NodeRng,
};
use casper_types::{PublicKey, U512};

/// Top-level event for the reactor.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, From, Serialize)]
#[must_use]
pub enum Event {
    /// Network event.
    #[from]
    Network(network::Event<Message>),

    /// Small Network event.
    #[from]
    SmallNetwork(small_network::Event<Message>),

    /// Storage event.
    #[from]
    Storage(#[serde(skip_serializing)] storage::Event),

    #[from]
    /// REST server event.
    RestServer(#[serde(skip_serializing)] rest_server::Event),

    #[from]
    /// Event stream server event.
    EventStreamServer(#[serde(skip_serializing)] event_stream_server::Event),

    /// Metrics request.
    #[from]
    MetricsRequest(#[serde(skip_serializing)] MetricsRequest),

    #[from]
    /// Chainspec Loader event.
    ChainspecLoader(#[serde(skip_serializing)] chainspec_loader::Event),

    /// Chainspec info request
    #[from]
    ChainspecLoaderRequest(#[serde(skip_serializing)] ChainspecLoaderRequest),

    /// Network info request.
    #[from]
    NetworkInfoRequest(#[serde(skip_serializing)] NetworkInfoRequest<NodeId>),

    /// Linear chain fetcher event.
    #[from]
    BlockFetcher(#[serde(skip_serializing)] fetcher::Event<Block>),

    /// Linear chain (by height) fetcher event.
    #[from]
    BlockByHeightFetcher(#[serde(skip_serializing)] fetcher::Event<BlockByHeight>),

    /// Deploy fetcher event.
    #[from]
    DeployFetcher(#[serde(skip_serializing)] fetcher::Event<Deploy>),

    /// Deploy acceptor event.
    #[from]
    DeployAcceptor(#[serde(skip_serializing)] deploy_acceptor::Event),

    /// Block validator event.
    #[from]
    BlockValidator(#[serde(skip_serializing)] block_validator::Event<NodeId>),

    /// Linear chain event.
    #[from]
    LinearChainSync(#[serde(skip_serializing)] linear_chain_sync::Event<NodeId>),

    /// Contract Runtime event.
    #[from]
    ContractRuntime(#[serde(skip_serializing)] ContractRuntimeRequest),

    /// Linear chain event.
    #[from]
    LinearChain(#[serde(skip_serializing)] linear_chain::Event<NodeId>),

    /// Address gossiper event.
    #[from]
    AddressGossiper(gossiper::Event<GossipedAddress>),

    /// Requests.
    /// Linear chain block by hash fetcher request.
    #[from]
    BlockFetcherRequest(#[serde(skip_serializing)] FetcherRequest<NodeId, Block>),

    /// Linear chain block by height fetcher request.
    #[from]
    BlockByHeightFetcherRequest(#[serde(skip_serializing)] FetcherRequest<NodeId, BlockByHeight>),

    /// Deploy fetcher request.
    #[from]
    DeployFetcherRequest(#[serde(skip_serializing)] FetcherRequest<NodeId, Deploy>),

    /// Block validation request.
    #[from]
    BlockValidatorRequest(#[serde(skip_serializing)] BlockValidationRequest<NodeId>),

    /// Block proposer request.
    #[from]
    BlockProposerRequest(#[serde(skip_serializing)] BlockProposerRequest),

    /// Request for state storage.
    #[from]
    StateStoreRequest(#[serde(skip_serializing)] StateStoreRequest),

    // Announcements
    /// A control announcement.
    #[from]
    ControlAnnouncement(ControlAnnouncement),

    /// Network announcement.
    #[from]
    NetworkAnnouncement(#[serde(skip_serializing)] NetworkAnnouncement<NodeId, Message>),

    /// Block executor announcement.
    #[from]
    ContractRuntimeAnnouncement(#[serde(skip_serializing)] ContractRuntimeAnnouncement),

    /// Address Gossiper announcement.
    #[from]
    AddressGossiperAnnouncement(#[serde(skip_serializing)] GossiperAnnouncement<GossipedAddress>),

    /// DeployAcceptor announcement.
    #[from]
    DeployAcceptorAnnouncement(#[serde(skip_serializing)] DeployAcceptorAnnouncement<NodeId>),

    /// Linear chain announcement.
    #[from]
    LinearChainAnnouncement(#[serde(skip_serializing)] LinearChainAnnouncement),

    /// Chainspec loader announcement.
    #[from]
    ChainspecLoaderAnnouncement(#[serde(skip_serializing)] ChainspecLoaderAnnouncement),

    /// Consensus request.
    #[from]
    ConsensusRequest(#[serde(skip_serializing)] ConsensusRequest),
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

impl From<LinearChainRequest<NodeId>> for Event {
    fn from(req: LinearChainRequest<NodeId>) -> Self {
        Event::LinearChain(linear_chain::Event::Request(req))
    }
}

impl From<StorageRequest> for Event {
    fn from(request: StorageRequest) -> Self {
        Event::Storage(request.into())
    }
}

impl From<NetworkRequest<NodeId, Message>> for Event {
    fn from(request: NetworkRequest<NodeId, Message>) -> Self {
        if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_ok() {
            Event::Network(network::Event::from(request))
        } else {
            Event::SmallNetwork(small_network::Event::from(request))
        }
    }
}

impl From<NetworkRequest<NodeId, gossiper::Message<GossipedAddress>>> for Event {
    fn from(request: NetworkRequest<NodeId, gossiper::Message<GossipedAddress>>) -> Self {
        Event::SmallNetwork(small_network::Event::from(
            request.map_payload(Message::from),
        ))
    }
}

impl From<RestRequest<NodeId>> for Event {
    fn from(request: RestRequest<NodeId>) -> Self {
        Event::RestServer(rest_server::Event::RestRequest(request))
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Network(event) => write!(f, "network: {}", event),
            Event::SmallNetwork(event) => write!(f, "small network: {}", event),
            Event::NetworkAnnouncement(event) => write!(f, "network announcement: {}", event),
            Event::Storage(request) => write!(f, "storage: {}", request),
            Event::RestServer(event) => write!(f, "rest server: {}", event),
            Event::EventStreamServer(event) => write!(f, "event stream server: {}", event),
            Event::MetricsRequest(req) => write!(f, "metrics request: {}", req),
            Event::ChainspecLoader(event) => write!(f, "chainspec loader: {}", event),
            Event::ChainspecLoaderRequest(req) => write!(f, "chainspec loader request: {}", req),
            Event::NetworkInfoRequest(req) => write!(f, "network info request: {}", req),
            Event::BlockFetcherRequest(request) => write!(f, "block fetcher request: {}", request),
            Event::BlockValidatorRequest(request) => {
                write!(f, "block validator request: {}", request)
            }
            Event::DeployFetcherRequest(request) => {
                write!(f, "deploy fetcher request: {}", request)
            }
            Event::LinearChainSync(event) => write!(f, "linear chain: {}", event),
            Event::BlockFetcher(event) => write!(f, "block fetcher: {}", event),
            Event::BlockByHeightFetcherRequest(request) => {
                write!(f, "block by height fetcher request: {}", request)
            }
            Event::BlockValidator(event) => write!(f, "block validator event: {}", event),
            Event::DeployFetcher(event) => write!(f, "deploy fetcher event: {}", event),
            Event::BlockProposerRequest(req) => write!(f, "block proposer request: {}", req),
            Event::ContractRuntime(event) => write!(f, "contract runtime event: {:?}", event),
            Event::LinearChain(event) => write!(f, "linear chain event: {}", event),
            Event::ContractRuntimeAnnouncement(announcement) => {
                write!(f, "block executor announcement: {}", announcement)
            }
            Event::AddressGossiper(event) => write!(f, "address gossiper: {}", event),
            Event::AddressGossiperAnnouncement(ann) => {
                write!(f, "address gossiper announcement: {}", ann)
            }
            Event::BlockByHeightFetcher(event) => {
                write!(f, "block by height fetcher event: {}", event)
            }
            Event::DeployAcceptorAnnouncement(ann) => {
                write!(f, "deploy acceptor announcement: {}", ann)
            }
            Event::DeployAcceptor(event) => write!(f, "deploy acceptor: {}", event),
            Event::ControlAnnouncement(ctrl_ann) => write!(f, "control: {}", ctrl_ann),
            Event::LinearChainAnnouncement(ann) => write!(f, "linear chain announcement: {}", ann),
            Event::ChainspecLoaderAnnouncement(ann) => {
                write!(f, "chainspec loader announcement: {}", ann)
            }
            Event::StateStoreRequest(req) => write!(f, "state store request: {}", req),
            Event::ConsensusRequest(req) => write!(f, "consensus request: {:?}", req),
        }
    }
}

/// Joining node reactor.
#[derive(DataSize)]
pub struct Reactor {
    root: PathBuf,
    metrics: Metrics,
    network: Network<Event, Message>,
    small_network: SmallNetwork<Event, Message>,
    address_gossiper: Gossiper<GossipedAddress, Event>,
    config: participating::Config,
    chainspec_loader: ChainspecLoader,
    storage: Storage,
    contract_runtime: ContractRuntime,
    linear_chain_fetcher: Fetcher<Block>,
    linear_chain_sync: LinearChainSync<NodeId>,
    block_validator: BlockValidator<NodeId>,
    deploy_fetcher: Fetcher<Deploy>,
    linear_chain: linear_chain::LinearChainComponent<NodeId>,
    // Handles request for linear chain block by height.
    block_by_height_fetcher: Fetcher<BlockByHeight>,
    pub(super) block_header_by_hash_fetcher: Fetcher<BlockHeader>,
    pub(super) block_header_with_metadata_fetcher: Fetcher<BlockHeaderWithMetadata>,
    #[data_size(skip)]
    deploy_acceptor: DeployAcceptor,
    #[data_size(skip)]
    event_queue_metrics: EventQueueMetrics,
    #[data_size(skip)]
    rest_server: RestServer,
    #[data_size(skip)]
    event_stream_server: EventStreamServer,
    // Attach memory metrics for the joiner.
    #[data_size(skip)] // Never allocates data on the heap.
    memory_metrics: MemoryMetrics,
}

impl reactor::Reactor for Reactor {
    type Event = Event;

    // The "configuration" is in fact the whole state of the initializer reactor, which we
    // deconstruct and reuse.
    type Config = WithDir<initializer::Reactor>;
    type Error = Error;

    fn new(
        initializer: Self::Config,
        registry: &Registry,
        event_queue: EventQueueHandle<Self::Event>,
        _rng: &mut NodeRng,
    ) -> Result<(Self, Effects<Self::Event>), Self::Error> {
        let (root, initializer) = initializer.into_parts();

        let initializer::Reactor {
            config,
            chainspec_loader,
            storage,
            mut contract_runtime,
            small_network_identity,
            network_identity,
        } = initializer;

        // TODO: Remove wrapper around Reactor::Config instead.
        let (_, config) = config.into_parts();

        let memory_metrics = MemoryMetrics::new(registry.clone())?;

        let event_queue_metrics = EventQueueMetrics::new(registry.clone(), event_queue)?;

        let metrics = Metrics::new(registry.clone());

        let network_config = network::Config::from(&config.network);
        let (network, network_effects) = Network::new(
            event_queue,
            network_config,
            registry,
            network_identity,
            chainspec_loader.chainspec(),
        )?;
        let (small_network, small_network_effects) = SmallNetwork::new(
            event_queue,
            config.network.clone(),
            Some(WithDir::new(&root, &config.consensus)),
            registry,
            small_network_identity,
            chainspec_loader.chainspec().as_ref(),
            None,
        )?;

        let linear_chain_fetcher = Fetcher::new("linear_chain", config.fetcher, registry)?;

        let mut effects = reactor::wrap_effects(Event::Network, network_effects);
        effects.extend(reactor::wrap_effects(
            Event::SmallNetwork,
            small_network_effects,
        ));

        let address_gossiper =
            Gossiper::new_for_complete_items("address_gossiper", config.gossip, registry)?;

        let effect_builder = EffectBuilder::new(event_queue);

        let init_hash = config
            .node
            .trusted_hash
            .or_else(|| chainspec_loader.initial_block_hash());

        match init_hash {
            None => {
                let chainspec = chainspec_loader.chainspec();
                let era_duration = chainspec.core_config.era_duration;
                if let Some(start_time) = chainspec
                    .protocol_config
                    .activation_point
                    .genesis_timestamp()
                {
                    if Timestamp::now() > start_time + era_duration {
                        error!(
                            "Node started with no trusted hash after the expected end of \
                             the genesis era! Please specify a trusted hash and restart. \
                             Time: {}, End of genesis era: {}",
                            Timestamp::now(),
                            start_time + era_duration
                        );
                        panic!("should have trusted hash after genesis era")
                    }
                }
                info!("No synchronization of the linear chain will be done.")
            }
            Some(hash) => info!("Synchronizing linear chain from: {:?}", hash),
        }

        let protocol_version = &chainspec_loader.chainspec().protocol_config.version;
        let rest_server = RestServer::new(
            config.rest_server.clone(),
            effect_builder,
            *protocol_version,
        )?;

        let event_stream_server = EventStreamServer::new(
            config.event_stream_server.clone(),
            storage.root_path().to_path_buf(),
            *protocol_version,
            DeployGetter::new(effect_builder),
        )?;

        let block_validator = BlockValidator::new(Arc::clone(chainspec_loader.chainspec()));

        let deploy_fetcher = Fetcher::new("deploy", config.fetcher, registry)?;

        let block_by_height_fetcher = Fetcher::new("block_by_height", config.fetcher, registry)?;

        let block_header_and_finality_signatures_by_height_fetcher: Fetcher<
            BlockHeaderWithMetadata,
        > = Fetcher::new(
            "block_header_and_finality_signatures_by_height",
            config.fetcher,
            registry,
        )?;

        let block_header_by_hash_fetcher: Fetcher<BlockHeader> =
            Fetcher::new("block_header_by_hash", config.fetcher, registry)?;

        let deploy_acceptor =
            DeployAcceptor::new(config.deploy_acceptor, &*chainspec_loader.chainspec());

        contract_runtime.set_initial_state(chainspec_loader.initial_execution_pre_state());
        let linear_chain = linear_chain::LinearChainComponent::new(
            registry,
            *protocol_version,
            chainspec_loader.chainspec().core_config.auction_delay,
            chainspec_loader.chainspec().core_config.unbonding_delay,
        )?;

        let validator_weights: BTreeMap<PublicKey, U512> = chainspec_loader
            .chainspec()
            .network_config
            .chainspec_validator_stakes()
            .into_iter()
            .map(|(pk, motes)| (pk, motes.value()))
            .collect();
        let maybe_next_activation_point = chainspec_loader
            .next_upgrade()
            .map(|next_upgrade| next_upgrade.activation_point());
        let (linear_chain_sync, init_sync_effects) = LinearChainSync::new::<Event, Error>(
            registry,
            effect_builder,
            chainspec_loader.chainspec(),
            &storage,
            init_hash,
            chainspec_loader.initial_block().cloned(),
            validator_weights,
            maybe_next_activation_point,
            chainspec_loader.initial_execution_pre_state(),
        )?;

        effects.extend(reactor::wrap_effects(
            Event::LinearChainSync,
            init_sync_effects,
        ));
        effects.extend(reactor::wrap_effects(
            Event::ChainspecLoader,
            chainspec_loader.start_checking_for_upgrades(effect_builder),
        ));

        Ok((
            Self {
                root,
                metrics,
                network,
                small_network,
                address_gossiper,
                config,
                chainspec_loader,
                storage,
                contract_runtime,
                linear_chain_sync,
                linear_chain_fetcher,
                block_validator,
                deploy_fetcher,
                linear_chain,
                block_by_height_fetcher,
                block_header_by_hash_fetcher,
                block_header_with_metadata_fetcher:
                    block_header_and_finality_signatures_by_height_fetcher,
                deploy_acceptor,
                event_queue_metrics,
                rest_server,
                event_stream_server,
                memory_metrics,
            },
            effects,
        ))
    }

    fn dispatch_event(
        &mut self,
        effect_builder: EffectBuilder<Self::Event>,
        rng: &mut NodeRng,
        event: Self::Event,
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
            Event::ControlAnnouncement(ctrl_ann) => {
                unreachable!("unhandled control announcement: {}", ctrl_ann)
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::NewPeer(id)) => reactor::wrap_effects(
                Event::LinearChainSync,
                self.linear_chain_sync.handle_event(
                    effect_builder,
                    rng,
                    linear_chain_sync::Event::NewPeerConnected(id),
                ),
            ),
            Event::NetworkAnnouncement(NetworkAnnouncement::GossipOurAddress(gossiped_address)) => {
                let event = gossiper::Event::ItemReceived {
                    item_id: gossiped_address,
                    source: Source::<NodeId>::Ourself,
                };
                self.dispatch_event(effect_builder, rng, Event::AddressGossiper(event))
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::MessageReceived {
                sender,
                payload,
            }) => match payload {
                Message::GetResponse {
                    tag: Tag::Block,
                    serialized_item,
                } => {
                    let block = match bincode::deserialize(&serialized_item) {
                        Ok(block) => Box::new(block),
                        Err(err) => {
                            error!("failed to decode block from {}: {}", sender, err);
                            return Effects::new();
                        }
                    };
                    let event = fetcher::Event::GotRemotely {
                        item: block,
                        source: Source::Peer(sender),
                    };
                    self.dispatch_event(effect_builder, rng, Event::BlockFetcher(event))
                }
                Message::GetResponse {
                    tag: Tag::BlockByHeight,
                    serialized_item,
                } => {
                    let block_at_height: BlockByHeight =
                        match bincode::deserialize(&serialized_item) {
                            Ok(maybe_block) => maybe_block,
                            Err(err) => {
                                error!("failed to decode block from {}: {}", sender, err);
                                return Effects::new();
                            }
                        };

                    let event = match block_at_height {
                        BlockByHeight::Absent(block_height) => fetcher::Event::AbsentRemotely {
                            id: block_height,
                            peer: sender,
                        },
                        BlockByHeight::Block(block) => fetcher::Event::GotRemotely {
                            item: Box::new(BlockByHeight::Block(block)),
                            source: Source::Peer(sender),
                        },
                    };
                    self.dispatch_event(effect_builder, rng, Event::BlockByHeightFetcher(event))
                }
                Message::GetResponse {
                    tag: Tag::Deploy,
                    serialized_item,
                } => {
                    let deploy = match bincode::deserialize(&serialized_item) {
                        Ok(deploy) => Box::new(deploy),
                        Err(err) => {
                            error!("failed to decode deploy from {}: {}", sender, err);
                            return Effects::new();
                        }
                    };
                    let event = Event::DeployAcceptor(deploy_acceptor::Event::Accept {
                        deploy,
                        source: Source::Peer(sender),
                        responder: None,
                    });
                    self.dispatch_event(effect_builder, rng, event)
                }
                Message::AddressGossiper(message) => {
                    let event = Event::AddressGossiper(gossiper::Event::MessageReceived {
                        sender,
                        message,
                    });
                    self.dispatch_event(effect_builder, rng, event)
                }
                Message::FinalitySignature(_) => {
                    debug!("finality signatures not handled in joiner reactor");
                    Effects::new()
                }
                other => {
                    debug!(?other, "network announcement ignored.");
                    Effects::new()
                }
            },
            Event::DeployAcceptorAnnouncement(DeployAcceptorAnnouncement::AcceptedNewDeploy {
                deploy,
                source,
            }) => {
                let event = event_stream_server::Event::DeployAccepted(*deploy.id());
                let mut effects =
                    self.dispatch_event(effect_builder, rng, Event::EventStreamServer(event));

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
                deploy,
                source,
            }) => {
                let deploy_hash = *deploy.id();
                let peer = source;
                warn!(?deploy_hash, ?peer, "Invalid deploy received from a peer.");
                Effects::new()
            }
            Event::Storage(event) => reactor::wrap_effects(
                Event::Storage,
                self.storage.handle_event(effect_builder, rng, event),
            ),
            Event::BlockFetcherRequest(request) => {
                self.dispatch_event(effect_builder, rng, Event::BlockFetcher(request.into()))
            }
            Event::BlockValidatorRequest(request) => {
                self.dispatch_event(effect_builder, rng, Event::BlockValidator(request.into()))
            }
            Event::DeployAcceptor(event) => reactor::wrap_effects(
                Event::DeployAcceptor,
                self.deploy_acceptor
                    .handle_event(effect_builder, rng, event),
            ),
            Event::LinearChainSync(event) => reactor::wrap_effects(
                Event::LinearChainSync,
                self.linear_chain_sync
                    .handle_event(effect_builder, rng, event),
            ),
            Event::BlockFetcher(event) => reactor::wrap_effects(
                Event::BlockFetcher,
                self.linear_chain_fetcher
                    .handle_event(effect_builder, rng, event),
            ),
            Event::BlockValidator(event) => reactor::wrap_effects(
                Event::BlockValidator,
                self.block_validator
                    .handle_event(effect_builder, rng, event),
            ),
            Event::DeployFetcher(event) => reactor::wrap_effects(
                Event::DeployFetcher,
                self.deploy_fetcher.handle_event(effect_builder, rng, event),
            ),
            Event::BlockByHeightFetcher(event) => reactor::wrap_effects(
                Event::BlockByHeightFetcher,
                self.block_by_height_fetcher
                    .handle_event(effect_builder, rng, event),
            ),
            Event::DeployFetcherRequest(request) => {
                self.dispatch_event(effect_builder, rng, Event::DeployFetcher(request.into()))
            }
            Event::BlockByHeightFetcherRequest(request) => self.dispatch_event(
                effect_builder,
                rng,
                Event::BlockByHeightFetcher(request.into()),
            ),
            Event::ContractRuntime(event) => reactor::wrap_effects(
                Event::ContractRuntime,
                self.contract_runtime
                    .handle_event(effect_builder, rng, event),
            ),
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
            }) => self.dispatch_event(
                effect_builder,
                rng,
                Event::EventStreamServer(event_stream_server::Event::Step {
                    era_id,
                    execution_effect,
                }),
            ),
            Event::LinearChain(event) => reactor::wrap_effects(
                Event::LinearChain,
                self.linear_chain.handle_event(effect_builder, rng, event),
            ),
            Event::BlockProposerRequest(request) => {
                // Consensus component should not be trying to create new blocks during joining
                // phase.
                error!("ignoring block proposer request {}", request);
                Effects::new()
            }
            Event::AddressGossiper(event) => reactor::wrap_effects(
                Event::AddressGossiper,
                self.address_gossiper
                    .handle_event(effect_builder, rng, event),
            ),
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
                let mut effects = reactor::wrap_effects(
                    Event::EventStreamServer,
                    self.event_stream_server.handle_event(
                        effect_builder,
                        rng,
                        event_stream_server::Event::BlockAdded(block.clone()),
                    ),
                );
                let reactor_event =
                    Event::LinearChainSync(linear_chain_sync::Event::BlockHandled(block.clone()));
                effects.extend(self.dispatch_event(effect_builder, rng, reactor_event));
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
            Event::RestServer(event) => reactor::wrap_effects(
                Event::RestServer,
                self.rest_server.handle_event(effect_builder, rng, event),
            ),
            Event::EventStreamServer(event) => reactor::wrap_effects(
                Event::EventStreamServer,
                self.event_stream_server
                    .handle_event(effect_builder, rng, event),
            ),
            Event::MetricsRequest(req) => reactor::wrap_effects(
                Event::MetricsRequest,
                self.metrics.handle_event(effect_builder, rng, req),
            ),
            Event::ChainspecLoader(event) => reactor::wrap_effects(
                Event::ChainspecLoader,
                self.chainspec_loader
                    .handle_event(effect_builder, rng, event),
            ),
            Event::ChainspecLoaderRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::ChainspecLoader(req.into()))
            }
            Event::StateStoreRequest(req) => {
                self.dispatch_event(effect_builder, rng, Event::Storage(req.into()))
            }
            Event::NetworkInfoRequest(req) => {
                let event = if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_ok() {
                    Event::Network(network::Event::from(req))
                } else {
                    Event::SmallNetwork(small_network::Event::from(req))
                };
                self.dispatch_event(effect_builder, rng, event)
            }
            Event::ChainspecLoaderAnnouncement(
                ChainspecLoaderAnnouncement::UpgradeActivationPointRead(next_upgrade),
            ) => {
                let reactor_event = Event::ChainspecLoader(
                    chainspec_loader::Event::GotNextUpgrade(next_upgrade.clone()),
                );
                let mut effects = self.dispatch_event(effect_builder, rng, reactor_event);

                let reactor_event =
                    Event::LinearChainSync(linear_chain_sync::Event::GotUpgradeActivationPoint(
                        next_upgrade.activation_point(),
                    ));
                effects.extend(self.dispatch_event(effect_builder, rng, reactor_event));
                effects
            }
            // This is done to handle status requests from the RestServer
            Event::ConsensusRequest(ConsensusRequest::Status(responder)) => {
                // no consensus, respond with None
                responder.respond(None).ignore()
            }
        }
    }

    fn maybe_exit(&self) -> Option<ReactorExit> {
        if self.linear_chain_sync.stopped_for_upgrade() {
            Some(ReactorExit::ProcessShouldExit(ExitCode::Success))
        } else if self.linear_chain_sync.is_synced() {
            Some(ReactorExit::ProcessShouldContinue)
        } else {
            None
        }
    }

    fn update_metrics(&mut self, event_queue_handle: EventQueueHandle<Self::Event>) {
        self.memory_metrics.estimate(self);
        self.event_queue_metrics
            .record_event_queue_counts(&event_queue_handle);
    }
}

impl Reactor {
    /// Deconstructs the reactor into config useful for creating a Validator reactor. Shuts down
    /// the network, closing all incoming and outgoing connections, and frees up the listening
    /// socket.
    pub async fn into_participating_config(self) -> Result<ParticipatingInitConfig, Error> {
        let maybe_latest_block_header = self.linear_chain_sync.into_maybe_latest_block_header();
        // Clean the state of the linear_chain_sync before shutting it down.
        #[cfg(not(feature = "fast-sync"))]
        linear_chain_sync::clean_linear_chain_state(
            &self.storage,
            self.chainspec_loader.chainspec(),
        )?;
        let config = ParticipatingInitConfig {
            root: self.root,
            chainspec_loader: self.chainspec_loader,
            config: self.config,
            contract_runtime: self.contract_runtime,
            storage: self.storage,
            maybe_latest_block_header,
            event_stream_server: self.event_stream_server,
            small_network_identity: SmallNetworkIdentity::from(&self.small_network),
            network_identity: NetworkIdentity::from(&self.network),
        };
        self.network.finalize().await;
        self.small_network.finalize().await;
        self.rest_server.finalize().await;
        Ok(config)
    }
}

#[cfg(test)]
impl NetworkedReactor for Reactor {
    type NodeId = NodeId;
    fn node_id(&self) -> Self::NodeId {
        if env::var(ENABLE_LIBP2P_NET_ENV_VAR).is_err() {
            self.small_network.node_id()
        } else {
            self.network.node_id()
        }
    }
}

#[cfg(test)]
impl Reactor {
    /// Inspect storage.
    pub(crate) fn storage(&self) -> &Storage {
        &self.storage
    }
}
