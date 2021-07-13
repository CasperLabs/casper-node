#![cfg(test)]
use std::{
    collections::{BTreeSet, HashMap},
    fmt::{self, Debug, Display, Formatter},
    iter,
};

use derive_more::From;
use prometheus::Registry;
use rand::Rng;
use reactor::ReactorEvent;
use serde::Serialize;
use tempfile::TempDir;
use thiserror::Error;
use tokio::time;
use tracing::debug;

use casper_types::ProtocolVersion;

use super::*;
use crate::{
    components::{
        contract_runtime::{self, ContractRuntime},
        deploy_acceptor::{self, DeployAcceptor},
        in_memory_network::{self, InMemoryNetwork, NetworkController},
        storage::{self, Storage},
    },
    effect::{
        announcements::{
            ContractRuntimeAnnouncement, ControlAnnouncement, DeployAcceptorAnnouncement,
            GossiperAnnouncement, NetworkAnnouncement, RpcServerAnnouncement,
        },
        requests::{ConsensusRequest, ContractRuntimeRequest, LinearChainRequest},
        Responder,
    },
    protocol::Message as NodeMessage,
    reactor::{self, EventQueueHandle, Runner},
    testing::{
        network::{Network, NetworkedReactor},
        ConditionCheckReactor, TestRng,
    },
    types::{Chainspec, Deploy, NodeId, Tag},
    utils::{Loadable, WithDir},
    NodeRng,
};

/// Top-level event for the reactor.
#[derive(Debug, From, Serialize)]
#[must_use]
enum Event {
    #[from]
    Network(in_memory_network::Event<NodeMessage>),
    #[from]
    Storage(#[serde(skip_serializing)] storage::Event),
    #[from]
    DeployAcceptor(#[serde(skip_serializing)] deploy_acceptor::Event),
    #[from]
    DeployGossiper(super::Event<Deploy>),
    #[from]
    NetworkRequest(NetworkRequest<NodeId, NodeMessage>),
    #[from]
    ControlAnnouncement(ControlAnnouncement),
    #[from]
    NetworkAnnouncement(#[serde(skip_serializing)] NetworkAnnouncement<NodeId, NodeMessage>),
    #[from]
    RpcServerAnnouncement(#[serde(skip_serializing)] RpcServerAnnouncement),
    #[from]
    DeployAcceptorAnnouncement(#[serde(skip_serializing)] DeployAcceptorAnnouncement<NodeId>),
    #[from]
    DeployGossiperAnnouncement(#[serde(skip_serializing)] GossiperAnnouncement<Deploy>),
    #[from]
    ContractRuntime(#[serde(skip_serializing)] ContractRuntimeRequest),
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
        Event::Storage(storage::Event::from(request))
    }
}

impl From<NetworkRequest<NodeId, Message<Deploy>>> for Event {
    fn from(request: NetworkRequest<NodeId, Message<Deploy>>) -> Self {
        Event::NetworkRequest(request.map_payload(NodeMessage::from))
    }
}

impl From<ConsensusRequest> for Event {
    fn from(_request: ConsensusRequest) -> Self {
        unimplemented!("not implemented for gossiper tests")
    }
}

impl From<LinearChainRequest<NodeId>> for Event {
    fn from(_request: LinearChainRequest<NodeId>) -> Self {
        unimplemented!("not implemented for gossiper tests")
    }
}

impl From<ContractRuntimeAnnouncement> for Event {
    fn from(_request: ContractRuntimeAnnouncement) -> Self {
        unimplemented!("not implemented for gossiper tests")
    }
}

impl Display for Event {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Network(event) => write!(formatter, "event: {}", event),
            Event::Storage(event) => write!(formatter, "storage: {}", event),
            Event::DeployAcceptor(event) => write!(formatter, "deploy acceptor: {}", event),
            Event::DeployGossiper(event) => write!(formatter, "deploy gossiper: {}", event),
            Event::NetworkRequest(req) => write!(formatter, "network request: {}", req),
            Event::ControlAnnouncement(ctrl_ann) => write!(formatter, "control: {}", ctrl_ann),
            Event::NetworkAnnouncement(ann) => write!(formatter, "network announcement: {}", ann),
            Event::RpcServerAnnouncement(ann) => {
                write!(formatter, "api server announcement: {}", ann)
            }
            Event::DeployAcceptorAnnouncement(ann) => {
                write!(formatter, "deploy-acceptor announcement: {}", ann)
            }
            Event::DeployGossiperAnnouncement(ann) => {
                write!(formatter, "deploy-gossiper announcement: {}", ann)
            }
            Event::ContractRuntime(event) => {
                write!(formatter, "contract-runtime event: {:?}", event)
            }
        }
    }
}

/// Error type returned by the test reactor.
#[derive(Debug, Error)]
enum Error {
    #[error("prometheus (metrics) error: {0}")]
    Metrics(#[from] prometheus::Error),
}

struct Reactor {
    network: InMemoryNetwork<NodeMessage>,
    storage: Storage,
    deploy_acceptor: DeployAcceptor,
    deploy_gossiper: Gossiper<Deploy, Event>,
    contract_runtime: ContractRuntime,
    _storage_tempdir: TempDir,
}

impl Drop for Reactor {
    fn drop(&mut self) {
        NetworkController::<NodeMessage>::remove_node(&self.network.node_id())
    }
}

impl reactor::Reactor for Reactor {
    type Event = Event;
    type Config = Config;
    type Error = Error;

    fn new(
        config: Self::Config,
        registry: &Registry,
        event_queue: EventQueueHandle<Self::Event>,
        rng: &mut NodeRng,
    ) -> Result<(Self, Effects<Self::Event>), Self::Error> {
        let network = NetworkController::create_node(event_queue, rng);

        let (storage_config, storage_tempdir) = storage::Config::default_for_tests();
        let storage_withdir = WithDir::new(storage_tempdir.path(), storage_config);
        let storage = Storage::new(
            &storage_withdir,
            None,
            ProtocolVersion::from_parts(1, 0, 0),
            false,
        )
        .unwrap();

        let contract_runtime_config = contract_runtime::Config::default();
        let contract_runtime = ContractRuntime::new(
            ProtocolVersion::from_parts(1, 0, 0),
            storage_withdir,
            &contract_runtime_config,
            registry,
        )
        .unwrap();

        let deploy_acceptor = DeployAcceptor::new(
            deploy_acceptor::Config::new(false),
            &Chainspec::from_resources("local"),
        );
        let deploy_gossiper = Gossiper::new_for_partial_items(
            "deploy_gossiper",
            config,
            get_deploy_from_storage,
            registry,
        )?;

        let reactor = Reactor {
            network,
            storage,
            deploy_acceptor,
            deploy_gossiper,
            contract_runtime,
            _storage_tempdir: storage_tempdir,
        };

        let effects = Effects::new();

        Ok((reactor, effects))
    }

    fn dispatch_event(
        &mut self,
        effect_builder: EffectBuilder<Self::Event>,
        rng: &mut NodeRng,
        event: Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Storage(event) => reactor::wrap_effects(
                Event::Storage,
                self.storage.handle_event(effect_builder, rng, event),
            ),
            Event::DeployAcceptor(event) => reactor::wrap_effects(
                Event::DeployAcceptor,
                self.deploy_acceptor
                    .handle_event(effect_builder, rng, event),
            ),
            Event::DeployGossiper(event) => reactor::wrap_effects(
                Event::DeployGossiper,
                self.deploy_gossiper
                    .handle_event(effect_builder, rng, event),
            ),
            Event::NetworkRequest(request) => reactor::wrap_effects(
                Event::Network,
                self.network
                    .handle_event(effect_builder, rng, request.into()),
            ),
            Event::ControlAnnouncement(ctrl_ann) => {
                unreachable!("unhandled control announcement: {}", ctrl_ann)
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::MessageReceived {
                sender,
                payload,
            }) => {
                let reactor_event = match payload {
                    NodeMessage::GetRequest {
                        tag: Tag::Deploy,
                        serialized_id,
                    } => {
                        // Note: This is copied almost verbatim from the validator reactor and
                        // needs to be refactored.

                        let deploy_hash = match bincode::deserialize(&serialized_id) {
                            Ok(hash) => hash,
                            Err(error) => {
                                error!(
                                    "failed to decode {:?} from {}: {}",
                                    serialized_id, sender, error
                                );
                                return Effects::new();
                            }
                        };

                        match self
                            .storage
                            .handle_deduplicated_legacy_direct_deploy_request(deploy_hash)
                        {
                            Some(serialized_item) => {
                                let message = NodeMessage::new_get_response_raw_unchecked::<Deploy>(
                                    serialized_item,
                                );
                                return effect_builder.send_message(sender, message).ignore();
                            }

                            None => {
                                debug!(%sender, %deploy_hash, "failed to get deploy (not found)");
                                return Effects::new();
                            }
                        }
                    }
                    NodeMessage::GetResponse {
                        tag: Tag::Deploy,
                        serialized_item,
                    } => {
                        let deploy = match bincode::deserialize(&serialized_item) {
                            Ok(deploy) => Box::new(deploy),
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
                    NodeMessage::DeployGossiper(message) => {
                        Event::DeployGossiper(super::Event::MessageReceived { sender, message })
                    }
                    msg => panic!("should not get {}", msg),
                };
                self.dispatch_event(effect_builder, rng, reactor_event)
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::GossipOurAddress(_)) => {
                unreachable!("should not receive announcements of type GossipOurAddress");
            }
            Event::NetworkAnnouncement(NetworkAnnouncement::NewPeer(_)) => {
                // We do not care about new peers in the gossiper test.
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
                let event = super::Event::ItemReceived {
                    item_id: *deploy.id(),
                    source,
                };
                self.dispatch_event(effect_builder, rng, Event::DeployGossiper(event))
            }
            Event::DeployAcceptorAnnouncement(DeployAcceptorAnnouncement::InvalidDeploy {
                deploy: _,
                source: _,
            }) => Effects::new(),
            Event::DeployGossiperAnnouncement(_ann) => {
                // We do not care about deploy gossiper announcements in the gossiper test.
                Effects::new()
            }
            Event::Network(event) => reactor::wrap_effects(
                Event::Network,
                self.network.handle_event(effect_builder, rng, event),
            ),
            Event::ContractRuntime(event) => reactor::wrap_effects(
                Event::ContractRuntime,
                self.contract_runtime
                    .handle_event(effect_builder, rng, event),
            ),
        }
    }

    fn maybe_exit(&self) -> Option<crate::reactor::ReactorExit> {
        unimplemented!()
    }
}

impl NetworkedReactor for Reactor {
    type NodeId = NodeId;

    fn node_id(&self) -> NodeId {
        self.network.node_id()
    }
}

fn announce_deploy_received(
    deploy: Box<Deploy>,
    responder: Option<Responder<Result<(), deploy_acceptor::Error>>>,
) -> impl FnOnce(EffectBuilder<Event>) -> Effects<Event> {
    |effect_builder: EffectBuilder<Event>| {
        effect_builder
            .announce_deploy_received(deploy, responder)
            .ignore()
    }
}

async fn run_gossip(rng: &mut TestRng, network_size: usize, deploy_count: usize) {
    const TIMEOUT: Duration = Duration::from_secs(20);
    const QUIET_FOR: Duration = Duration::from_millis(50);

    NetworkController::<NodeMessage>::create_active();
    let mut network = Network::<Reactor>::new();

    // Add `network_size` nodes.
    let node_ids = network.add_nodes(rng, network_size).await;

    // Create `deploy_count` random deploys.
    let (all_deploy_hashes, mut deploys): (BTreeSet<_>, Vec<_>) = iter::repeat_with(|| {
        let deploy = Box::new(Deploy::random(rng));
        (*deploy.id(), deploy)
    })
    .take(deploy_count)
    .unzip();

    // Give each deploy to a randomly-chosen node to be gossiped.
    for deploy in deploys.drain(..) {
        let index: usize = rng.gen_range(0..network_size);
        network
            .process_injected_effect_on(&node_ids[index], announce_deploy_received(deploy, None))
            .await;
    }

    // Check every node has every deploy stored locally.
    let all_deploys_held = |nodes: &HashMap<NodeId, Runner<ConditionCheckReactor<Reactor>>>| {
        nodes.values().all(|runner| {
            let hashes = runner.reactor().inner().storage.get_all_deploy_hashes();
            all_deploy_hashes == hashes
        })
    };
    network.settle_on(rng, all_deploys_held, TIMEOUT).await;

    // Ensure all responders are called before dropping the network.
    network.settle(rng, QUIET_FOR, TIMEOUT).await;

    NetworkController::<NodeMessage>::remove_active();
}

#[tokio::test]
async fn should_gossip() {
    const NETWORK_SIZES: [usize; 3] = [2, 5, 20];
    const DEPLOY_COUNTS: [usize; 3] = [1, 10, 30];

    let mut rng = crate::new_rng();

    for network_size in &NETWORK_SIZES {
        for deploy_count in &DEPLOY_COUNTS {
            run_gossip(&mut rng, *network_size, *deploy_count).await
        }
    }
}

#[tokio::test]
async fn should_get_from_alternate_source() {
    const NETWORK_SIZE: usize = 3;
    const POLL_DURATION: Duration = Duration::from_millis(10);
    const TIMEOUT: Duration = Duration::from_secs(2);

    NetworkController::<NodeMessage>::create_active();
    let mut network = Network::<Reactor>::new();
    let mut rng = crate::new_rng();

    // Add `NETWORK_SIZE` nodes.
    let node_ids = network.add_nodes(&mut rng, NETWORK_SIZE).await;

    // Create random deploy.
    let deploy = Box::new(Deploy::random(&mut rng));
    let deploy_id = *deploy.id();

    // Give the deploy to nodes 0 and 1 to be gossiped.
    for node_id in node_ids.iter().take(2) {
        network
            .process_injected_effect_on(node_id, announce_deploy_received(deploy.clone(), None))
            .await;
    }

    // Run node 0 until it has sent the gossip request then remove it from the network.
    let made_gossip_request = |event: &Event| -> bool {
        matches!(event, Event::NetworkRequest(NetworkRequest::Gossip { .. }))
    };
    network
        .crank_until(&node_ids[0], &mut rng, made_gossip_request, TIMEOUT)
        .await;
    assert!(network.remove_node(&node_ids[0]).is_some());
    debug!("removed node {}", &node_ids[0]);

    // Run node 2 until it receives and responds to the gossip request from node 0.
    let node_id_0 = node_ids[0];
    let sent_gossip_response = move |event: &Event| -> bool {
        match event {
            Event::NetworkRequest(NetworkRequest::SendMessage { dest, payload, .. }) => {
                if let NodeMessage::DeployGossiper(Message::GossipResponse { .. }) = **payload {
                    **dest == node_id_0
                } else {
                    false
                }
            }
            _ => false,
        }
    };
    network
        .crank_until(&node_ids[2], &mut rng, sent_gossip_response, TIMEOUT)
        .await;

    // Run nodes 1 and 2 until settled.  Node 2 will be waiting for the deploy from node 0.
    network.settle(&mut rng, POLL_DURATION, TIMEOUT).await;

    // Advance time to trigger node 2's timeout causing it to request the deploy from node 1.
    let secs_to_advance = Config::default().get_remainder_timeout_secs();
    time::pause();
    time::advance(Duration::from_secs(secs_to_advance)).await;
    time::resume();
    debug!("advanced time by {} secs", secs_to_advance);

    // Check node 0 has the deploy stored locally.
    let deploy_held = |nodes: &HashMap<NodeId, Runner<ConditionCheckReactor<Reactor>>>| {
        let runner = nodes.get(&node_ids[2]).unwrap();
        runner
            .reactor()
            .inner()
            .storage
            .get_deploy_by_hash(deploy_id)
            .map(|retrieved_deploy| retrieved_deploy == *deploy)
            .unwrap_or_default()
    };
    network.settle_on(&mut rng, deploy_held, TIMEOUT).await;

    NetworkController::<NodeMessage>::remove_active();
}

#[tokio::test]
async fn should_timeout_gossip_response() {
    const PAUSE_DURATION: Duration = Duration::from_millis(50);
    const TIMEOUT: Duration = Duration::from_secs(2);

    NetworkController::<NodeMessage>::create_active();
    let mut network = Network::<Reactor>::new();
    let mut rng = crate::new_rng();

    // The target number of peers to infect with a given piece of data.
    let infection_target = Config::default().infection_target();

    // Add `infection_target + 1` nodes.
    let mut node_ids = network
        .add_nodes(&mut rng, infection_target as usize + 1)
        .await;

    // Create random deploy.
    let deploy = Box::new(Deploy::random(&mut rng));
    let deploy_id = *deploy.id();

    // Give the deploy to node 0 to be gossiped.
    network
        .process_injected_effect_on(&node_ids[0], announce_deploy_received(deploy.clone(), None))
        .await;

    // Run node 0 until it has sent the gossip requests.
    let made_gossip_request = |event: &Event| -> bool {
        matches!(
            event,
            Event::DeployGossiper(super::Event::GossipedTo { .. })
        )
    };
    network
        .crank_until(&node_ids[0], &mut rng, made_gossip_request, TIMEOUT)
        .await;
    // Give node 0 time to set the timeouts before advancing the clock.
    time::sleep(PAUSE_DURATION).await;

    // Replace all nodes except node 0 with new nodes.
    for node_id in node_ids.drain(1..) {
        assert!(network.remove_node(&node_id).is_some());
        debug!("removed node {}", node_id);
    }
    for _ in 0..infection_target {
        let (node_id, _runner) = network.add_node(&mut rng).await.unwrap();
        node_ids.push(node_id);
    }

    // Advance time to trigger node 0's timeout causing it to gossip to the new nodes.
    let secs_to_advance = Config::default().gossip_request_timeout_secs();
    time::pause();
    time::advance(Duration::from_secs(secs_to_advance)).await;
    time::resume();
    debug!("advanced time by {} secs", secs_to_advance);

    // Check every node has every deploy stored locally.
    let deploy_held = |nodes: &HashMap<NodeId, Runner<ConditionCheckReactor<Reactor>>>| {
        nodes.values().all(|runner| {
            runner
                .reactor()
                .inner()
                .storage
                .get_deploy_by_hash(deploy_id)
                .map(|retrieved_deploy| retrieved_deploy == *deploy)
                .unwrap_or_default()
        })
    };
    network.settle_on(&mut rng, deploy_held, TIMEOUT).await;

    NetworkController::<NodeMessage>::remove_active();
}
