use std::{
    collections::HashMap,
    env,
    fmt::{Debug, Display},
    time::Duration,
};

use libp2p::kad::kbucket::K_VALUE;
use rand::{distributions::Standard, Rng};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    effect::EffectExt, reactor::Runner, testing, testing::TestRng, types::NodeId, Chainspec,
};
use casper_node_macros::reactor;
use testing::{init_logging, network::NetworkedReactor, ConditionCheckReactor};

use super::ENABLE_LIBP2P_ENV_VAR;

// Reactor for load testing, whose networking component just sends dummy payloads around.
reactor!(LoadTestingReactor {
  type Config = TestReactorConfig;

  components: {
      net = has_effects Network::<LoadTestingReactorEvent, DummyPayload>(
        event_queue, cfg.network_config, &cfg.chainspec, false
      );
      collector = infallible Collector::<DummyPayload>();
  }

  events: {
      net = Event<DummyPayload>;
      collector = Event<DummyPayload>;
  }

  requests: {
      NetworkRequest<NodeId, DummyPayload> -> net;
  }

  announcements: {
      NetworkAnnouncement<NodeId, DummyPayload> -> [collector];
  }
});

impl NetworkedReactor for LoadTestingReactor {
    type NodeId = NodeId;

    fn node_id(&self) -> Self::NodeId {
        self.net.node_id()
    }
}

/// Configuration for the test reactor.
#[derive(Debug)]
pub struct TestReactorConfig {
    /// The fixed chainspec used in testing.
    chainspec: Chainspec,
    /// Network configuration used in testing.
    network_config: crate::components::network::Config,
}

/// A dummy payload.
#[derive(Clone, Eq, Deserialize, PartialEq, Serialize)]
pub struct DummyPayload(Vec<u8>);

impl DummyPayload {
    fn random_with_size(rng: &mut TestRng, sz: usize) -> Self {
        DummyPayload(rng.sample_iter(Standard).take(sz).collect())
    }
}

impl Debug for DummyPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "payload ({} bytes: {:?}...)",
            self.0.len(),
            &self.0[0..self.0.len().min(10)]
        )
    }
}

impl Display for DummyPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self, f)
    }
}

#[tokio::test]
async fn send_large_message_across_network() {
    init_logging();

    if env::var(ENABLE_LIBP2P_ENV_VAR).is_err() {
        eprintln!("{} not set, skipping test", ENABLE_LIBP2P_ENV_VAR);
        return;
    }

    let node_count: usize = 30;

    // Fully connecting a 20 node network takes ~ 3 seconds. This should be ample time for gossip
    // and connecting.
    let timeout = Duration::from_secs(20);
    let large_size: usize = 512;

    let mut rng = crate::new_rng();

    // Port for first node, other will connect to it.
    let first_node_port = testing::unused_port_on_localhost() + 1;

    let mut net = testing::network::Network::<LoadTestingReactor>::new();
    let chainspec = Chainspec::random(&mut rng);

    // Create the root node.
    let cfg = TestReactorConfig {
        chainspec: chainspec.clone(),
        network_config: crate::components::network::Config::default_local_net_first_node(
            first_node_port,
        ),
    };

    net.add_node_with_config(cfg, &mut rng).await.unwrap();

    // Create `node_count-1` additional node instances.
    for _ in 1..node_count {
        let cfg = TestReactorConfig {
            chainspec: chainspec.clone(),
            network_config: crate::components::network::Config::default_local_net(first_node_port),
        };

        net.add_node_with_config(cfg, &mut rng).await.unwrap();
    }

    info!("Network setup, waiting for discovery to complete");
    net.settle_on(&mut rng, network_online, timeout).await;
    info!("Discovery complete");

    // At this point each node has at least one other peer. Assuming no split, we can now start
    // gossiping a large payloads. We gossip one on each node.
    let node_ids: Vec<_> = net.nodes().keys().cloned().collect();
    for (index, sender) in node_ids.iter().enumerate() {
        let dummy_payload = DummyPayload::random_with_size(&mut rng, large_size);

        // Calling `broadcast_message` actually triggers libp2p gossping.
        net.process_injected_effect_on(sender, |effect_builder| {
            effect_builder
                .broadcast_message(dummy_payload.clone())
                .ignore()
        })
        .await;

        info!(?sender, payload = %dummy_payload, round=index, total=node_ids.len(),
              "Started broadcast/gossip of payload, waiting for all nodes to receive it");
        net.settle_on(
            &mut rng,
            others_received(&dummy_payload, sender.clone()),
            timeout,
        )
        .await;
        info!(?sender, "Completed gossip test for sender")
    }
}

/// Checks if all nodes are connected to at least one other node.
pub fn network_online(
    nodes: &HashMap<NodeId, Runner<ConditionCheckReactor<LoadTestingReactor>>>,
) -> bool {
    assert!(
        nodes.len() >= 2,
        "cannot check for an online network with less than 3 nodes"
    );

    let k_value = usize::from(K_VALUE);

    // Sanity check of K_VALUE.
    assert!(
        k_value >= 7,
        "K_VALUE is really small, expected it to be at least 7"
    );

    // The target of known nodes to go for. This has a hard bound of `K_VALUE`, since if all nodes
    // end up in the same bucket, we will start evicting them. In general, we go for K_VALUE/2 for
    // reasonable interconnection, or the network size - 1, which is another bound.
    let known_nodes_target = (k_value / 2).min(nodes.len() - 1);

    // Checks if all nodes have reached the known nodes target.
    nodes
        .values()
        .all(|runner| runner.reactor().inner().net.seen_peers().len() >= known_nodes_target)
}

/// Checks whether or not every node except `sender` on the network received the given payload.
pub fn others_received<'a>(
    payload: &'a DummyPayload,
    sender: NodeId,
) -> impl Fn(&HashMap<NodeId, Runner<ConditionCheckReactor<LoadTestingReactor>>>) -> bool + 'a {
    move |nodes| {
        nodes
            .values()
            // We're only interested in the inner reactor.
            .map(|runner| runner.reactor().inner())
            // Skip the sender.
            .filter(|reactor| reactor.node_id() != sender)
            // Ensure others all have received the payload.
            .all(|reactor| reactor.collector.payloads.contains(payload))
    }
}