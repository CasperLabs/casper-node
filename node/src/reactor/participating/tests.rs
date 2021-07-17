use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::bail;
use log::info;
use num::Zero;
use num_rational::Ratio;
use rand::Rng;
use tempfile::TempDir;

use casper_execution_engine::{core::engine_state::query::GetBidsRequest, shared::motes::Motes};
use casper_types::{
    system::auction::{Bids, DelegationRate},
    EraId, PublicKey, SecretKey, U512,
};

use crate::{
    components::{consensus, gossiper, small_network, storage},
    crypto::AsymmetricKeyExt,
    reactor::{initializer, joiner, participating, ReactorExit, Runner},
    testing::{self, network::Network, TestRng},
    types::{
        chainspec::{AccountConfig, AccountsConfig, ValidatorConfig},
        ActivationPoint, BlockHeader, Chainspec, Timestamp,
    },
    utils::{External, Loadable, WithDir, RESOURCES_PATH},
    NodeRng,
};

struct TestChain {
    // Keys that validator instances will use, can include duplicates
    keys: Vec<Arc<SecretKey>>,
    storages: Vec<TempDir>,
    chainspec: Arc<Chainspec>,
}

type Nodes = crate::testing::network::Nodes<participating::Reactor>;

impl TestChain {
    /// Instantiates a new test chain configuration.
    ///
    /// Generates secret keys for `size` validators and creates a matching chainspec.
    fn new(rng: &mut TestRng, size: usize) -> Self {
        let keys: Vec<Arc<SecretKey>> = (0..size)
            .map(|_| Arc::new(SecretKey::random(rng)))
            .collect();
        let stakes = keys
            .iter()
            .map(|secret_key| {
                // We use very large stakes so we would catch overflow issues.
                let stake = U512::from(rng.gen_range(100..999)) * U512::from(u128::MAX);
                let secret_key = secret_key.clone();
                (PublicKey::from(&*secret_key), stake)
            })
            .collect();
        Self::new_with_keys(rng, keys, stakes)
    }

    /// Instantiates a new test chain configuration.
    ///
    /// Takes a vector of bonded keys with specified bond amounts.
    fn new_with_keys(
        rng: &mut TestRng,
        keys: Vec<Arc<SecretKey>>,
        stakes: BTreeMap<PublicKey, U512>,
    ) -> Self {
        // Load the `local` chainspec.
        let mut chainspec = Chainspec::from_resources("local");

        // Override accounts with those generated from the keys.
        let accounts = stakes
            .into_iter()
            .map(|(public_key, bonded_amount)| {
                let validator_config =
                    ValidatorConfig::new(Motes::new(bonded_amount), DelegationRate::zero());
                AccountConfig::new(
                    public_key,
                    Motes::new(U512::from(rng.gen_range(10000..99999999))),
                    Some(validator_config),
                )
            })
            .collect();
        let delegators = vec![];
        chainspec.network_config.accounts_config = AccountsConfig::new(accounts, delegators);

        // Make the genesis timestamp 45 seconds from now, to allow for all validators to start up.
        chainspec.protocol_config.activation_point =
            ActivationPoint::Genesis(Timestamp::now() + 45000.into());

        chainspec.core_config.minimum_era_height = 1;
        chainspec.highway_config.finality_threshold_fraction = Ratio::new(34, 100);
        chainspec.core_config.era_duration = 10.into();
        chainspec.core_config.auction_delay = 1;
        chainspec.core_config.unbonding_delay = 3;

        TestChain {
            keys,
            chainspec: Arc::new(chainspec),
            storages: Vec::new(),
        }
    }

    /// Creates an initializer/validator configuration for the `idx`th validator.
    fn create_node_config(&mut self, idx: usize, first_node_port: u16) -> participating::Config {
        // Set the network configuration.
        let mut cfg = participating::Config {
            network: if idx == 0 {
                small_network::Config::default_local_net_first_node(first_node_port)
            } else {
                small_network::Config::default_local_net(first_node_port)
            },
            gossip: gossiper::Config::new_with_small_timeouts(),
            ..Default::default()
        };

        // ...and the secret key for our validator.
        cfg.consensus.secret_key_path = External::from_value(self.keys[idx].clone());

        // Additionally set up storage in a temporary directory.
        let (storage_cfg, temp_dir) = storage::Config::default_for_tests();
        cfg.consensus.highway.unit_hashes_folder = temp_dir.path().to_path_buf();
        self.storages.push(temp_dir);
        cfg.storage = storage_cfg;

        cfg
    }

    async fn create_initialized_network(
        &mut self,
        rng: &mut NodeRng,
    ) -> anyhow::Result<Network<participating::Reactor>> {
        let root = RESOURCES_PATH.join("local");

        let mut network: Network<participating::Reactor> = Network::new();
        let first_node_port = testing::unused_port_on_localhost();

        for idx in 0..self.keys.len() {
            let cfg = self.create_node_config(idx, first_node_port);

            // We create an initializer reactor here and run it to completion.
            let mut initializer_runner = Runner::<initializer::Reactor>::new_with_chainspec(
                (false, WithDir::new(root.clone(), cfg)),
                Arc::clone(&self.chainspec),
            )
            .await?;
            let reactor_exit = initializer_runner.run(rng).await;
            if reactor_exit != ReactorExit::ProcessShouldContinue {
                bail!("failed to initialize successfully");
            }

            // Now we can construct the actual node.
            let initializer = initializer_runner.drain_into_inner().await;
            let mut joiner_runner =
                Runner::<joiner::Reactor>::new(WithDir::new(root.clone(), initializer), rng)
                    .await?;
            let _ = joiner_runner.run(rng).await;

            let config = joiner_runner
                .drain_into_inner()
                .await
                .into_participating_config()
                .await?;

            network
                .add_node_with_config(config, rng)
                .await
                .expect("could not add node to reactor");
        }

        Ok(network)
    }
}

/// Given an era number, returns a predicate to check if all of the nodes are in the specified era.
fn is_in_era(era_id: EraId) -> impl Fn(&Nodes) -> bool {
    move |nodes: &Nodes| {
        nodes
            .values()
            .all(|runner| runner.reactor().inner().consensus().current_era() == era_id)
    }
}

/// Returns the bids at the given block.
fn get_bids(nodes: &Nodes, header: &BlockHeader) -> Bids {
    let correlation_id = Default::default();
    let request = GetBidsRequest::new(header.state_root_hash().clone().into());

    let runner = nodes.values().next().expect("missing nodes");
    let engine_state = runner.reactor().inner().contract_runtime().engine_state();
    let bids_result = engine_state
        .get_bids(correlation_id, request)
        .expect("get_bids failed");
    bids_result.bids().expect("no bids returned").clone()
}

#[tokio::test]
async fn run_participating_network() {
    testing::init_logging();

    let mut rng = crate::new_rng();

    // Instantiate a new chain with a fixed size.
    const NETWORK_SIZE: usize = 5;
    let mut chain = TestChain::new(&mut rng, NETWORK_SIZE);

    let mut net = chain
        .create_initialized_network(&mut rng)
        .await
        .expect("network initialization failed");

    // Wait for all nodes to agree on one era.
    net.settle_on(&mut rng, is_in_era(EraId::from(1)), Duration::from_secs(90))
        .await;

    net.settle_on(&mut rng, is_in_era(EraId::from(2)), Duration::from_secs(60))
        .await;
}

#[tokio::test]
async fn run_equivocator_network() {
    testing::init_logging();

    let mut rng = crate::new_rng();

    let alice_sk = Arc::new(SecretKey::random(&mut rng));
    let alice_pk = PublicKey::from(&*alice_sk);
    let size: usize = 2;
    let mut keys: Vec<Arc<SecretKey>> = (1..size)
        .map(|_| Arc::new(SecretKey::random(&mut rng)))
        .collect();
    let mut stakes: BTreeMap<PublicKey, U512> = keys
        .iter()
        .map(|secret_key| (PublicKey::from(&*secret_key.clone()), U512::from(100)))
        .collect();
    stakes.insert(PublicKey::from(&*alice_sk), U512::from(1));
    keys.push(alice_sk.clone());
    keys.push(alice_sk);

    let mut chain = TestChain::new_with_keys(&mut rng, keys, stakes.clone());
    let protocol_config = (&*chain.chainspec).into();

    let mut net = chain
        .create_initialized_network(&mut rng)
        .await
        .expect("network initialization failed");

    let timeout = Duration::from_secs(90);

    let mut switch_blocks = Vec::new();
    for era_number in 1..20 {
        let era_id = EraId::from(era_number);
        info!("Waiting for Era {} to begin", era_number);
        net.settle_on(&mut rng, is_in_era(era_id), timeout).await;

        // Collect new switch block headers.
        for runner in net.nodes().values() {
            let storage = runner.reactor().inner().storage();
            let header = storage
                .transactional_get_switch_block_by_era_id(era_number - 1)
                .expect("missing switch block")
                .take_header();
            assert_eq!(era_number - 1, header.era_id().value());
            if let Some(other_header) = switch_blocks.get(era_number as usize - 1) {
                assert_eq!(other_header, &header);
            } else {
                switch_blocks.push(header);
            }
        }

        let expected = [alice_pk.clone()];
        // Returns true if alice is listed as an equivocator in that block.
        let alice_is_equivocator = |header: &BlockHeader| {
            header.era_end().expect("missing era end").equivocators == expected
        };

        // Verify that nobody gets slashed, and Alice's bid becomes inactive, but only after she
        // equivocated.
        let bids = get_bids(net.nodes(), switch_blocks.last().unwrap());
        for (pk, stake) in &stakes {
            assert!(bids[pk].staked_amount() >= stake);
            assert!(*pk == alice_pk || !bids[pk].inactive());
        }
        assert_eq!(
            bids[&alice_pk].inactive(),
            switch_blocks.iter().any(alice_is_equivocator)
        );

        // Make sure we waited long enough for this test to include unbonding and dropping eras.
        let oldest_bonded_era_id = consensus::oldest_bonded_era(&protocol_config, era_id);
        let oldest_evidence_era_id =
            consensus::oldest_bonded_era(&protocol_config, oldest_bonded_era_id);
        if oldest_evidence_era_id.is_genesis() || era_number < 3 {
            continue;
        }

        // Wait at least two more eras after the equivocation has been detected.
        if switch_blocks[..(era_number as usize - 2)]
            .iter()
            .any(alice_is_equivocator)
        {
            break;
        }
    }

    // The auction delay is 1, so if Alice's equivocation was detected before the switch block in
    // era N, the switch block of era N should list her as faulty. Starting with the switch block
    // in era N + 1, she should be removed from the validator set, because she gets evicted in era
    // N + 2.
    // No era after N should have direct evidence against her: she got marked as faulty when era
    // N + 1 was initialized, so no other validator will cite her or process her units.
    loop {
        let header = switch_blocks.pop().expect("missing switch block");
        let validators = header
            .next_era_validator_weights()
            .expect("missing validator weights");
        if validators.contains_key(&alice_pk) {
            // We've found era N: This is the last switch block that still lists Alice as a
            // validator.
            let era_end = header.era_end().expect("missing era end");
            assert_eq!(*era_end.equivocators, [alice_pk.clone()]);
            return;
        } else {
            // We are in era N + 1 or later. There should be no direct evidence; that would mean
            // Alice equivocated twice.
            for runner in net.nodes().values() {
                let consensus = runner.reactor().inner().consensus();
                assert_eq!(
                    consensus.validators_with_evidence(header.era_id()),
                    Vec::<&PublicKey>::new()
                );
            }
        }
    }
}
