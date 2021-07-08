use casper_execution_engine::{
    core::engine_state::executable_deploy_item::ExecutableDeployItem, shared::gas::Gas,
};
use casper_types::{
    bytesrepr::Bytes, runtime_args, system::standard_payment::ARG_AMOUNT, RuntimeArgs, SecretKey,
};
use itertools::Itertools;

use super::*;
use crate::{
    crypto::AsymmetricKeyExt,
    testing::TestRng,
    types::{Deploy, DeployHash, TimeDiff},
};

const DEFAULT_TEST_GAS_PRICE: u64 = 1;

fn default_gas_payment() -> Gas {
    Gas::from(1u32)
}

fn generate_transfer(
    rng: &mut TestRng,
    timestamp: Timestamp,
    ttl: TimeDiff,
    dependencies: Vec<DeployHash>,
    payment_amount: Gas,
) -> Deploy {
    let gas_price = DEFAULT_TEST_GAS_PRICE;
    let secret_key = SecretKey::random(rng);
    let chain_name = "chain".to_string();

    let args = runtime_args! {
        ARG_AMOUNT => payment_amount.value()
    };
    let payment = ExecutableDeployItem::ModuleBytes {
        module_bytes: Bytes::new(),
        args,
    };

    let session = ExecutableDeployItem::Transfer {
        args: RuntimeArgs::new(),
    };

    Deploy::new(
        timestamp,
        ttl,
        gas_price,
        dependencies,
        chain_name,
        payment,
        session,
        &secret_key,
        None,
    )
}

fn generate_deploy(
    rng: &mut TestRng,
    timestamp: Timestamp,
    ttl: TimeDiff,
    dependencies: Vec<DeployHash>,
    payment_amount: Gas,
    gas_price: u64,
) -> Deploy {
    let secret_key = SecretKey::random(rng);
    let chain_name = "chain".to_string();
    let args = runtime_args! {
        ARG_AMOUNT => payment_amount.value()
    };
    let payment = ExecutableDeployItem::ModuleBytes {
        module_bytes: Bytes::new(),
        args,
    };
    let session = ExecutableDeployItem::ModuleBytes {
        module_bytes: Bytes::new(),
        args: RuntimeArgs::new(),
    };

    Deploy::new(
        timestamp,
        ttl,
        gas_price,
        dependencies,
        chain_name,
        payment,
        session,
        &secret_key,
        None,
    )
}

impl From<StorageRequest> for Event {
    fn from(_: StorageRequest) -> Self {
        // we never send a storage request in our unit tests, but if this does become
        // meaningful....
        unreachable!("no storage requests in block proposer unit tests")
    }
}

impl From<StateStoreRequest> for Event {
    fn from(_: StateStoreRequest) -> Self {
        unreachable!("no state store requests in block proposer unit tests")
    }
}

#[test]
fn should_add_and_take_deploys() {
    let creation_time = Timestamp::from(100);
    let ttl = TimeDiff::from(Duration::from_millis(100));
    let block_time1 = Timestamp::from(80);
    let block_time2 = Timestamp::from(120);
    let block_time3 = Timestamp::from(220);

    let mut proposer = BlockProposerReady::default();
    let mut rng = crate::new_rng();
    let deploy1 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy2 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy3 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy4 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );

    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time2, vec![]),
        vec![],
        true,
    );
    assert!(block.deploy_hashes().is_empty());
    assert!(block.transfer_hashes().is_empty());

    // add two deploys
    proposer.add_deploy(block_time2, Box::new(deploy1.clone()));
    proposer.add_deploy(block_time2, Box::new(deploy2.clone()));

    // if we try to create a block with a timestamp that is too early, we shouldn't get any
    // deploys
    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time1, vec![]),
        vec![],
        true,
    );
    assert!(block.deploy_hashes().is_empty());
    assert!(block.transfer_hashes().is_empty());

    // if we try to create a block with a timestamp that is too late, we shouldn't get any
    // deploys, either
    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time3, vec![]),
        vec![],
        true,
    );
    assert!(block.deploy_hashes().is_empty());
    assert!(block.transfer_hashes().is_empty());

    // take the deploys out
    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time2, vec![]),
        vec![],
        true,
    );
    assert!(block.transfer_hashes().is_empty());
    assert_eq!(block.deploy_hashes().len(), 2);
    assert!(block.deploy_hashes().contains(deploy1.id()));
    assert!(block.deploy_hashes().contains(deploy2.id()));

    // take the deploys out
    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time2, vec![]),
        vec![],
        true,
    );
    assert!(block.transfer_hashes().is_empty());
    assert_eq!(block.deploy_hashes().len(), 2);

    // but they shouldn't be returned if we include it in the past deploys
    let deploy_hashes = block.deploys_and_transfers_iter().collect_vec();
    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time2, vec![block]),
        vec![],
        true,
    );
    assert!(block.deploy_hashes().is_empty());
    assert!(block.transfer_hashes().is_empty());

    // finalize the block
    proposer.finalized_deploys(deploy_hashes.iter().copied());

    // add more deploys
    proposer.add_deploy(block_time2, Box::new(deploy3.clone()));
    proposer.add_deploy(block_time2, Box::new(deploy4.clone()));

    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time2, vec![]),
        vec![],
        true,
    );

    // since block 1 is now finalized, neither deploy1 nor deploy2 should be among the returned
    assert!(block.transfer_hashes().is_empty());
    assert_eq!(block.deploy_hashes().len(), 2);
    assert!(block.deploy_hashes().contains(deploy3.id()));
    assert!(block.deploy_hashes().contains(deploy4.id()));
}

#[test]
fn should_successfully_prune() {
    let expired_time = Timestamp::from(201);
    let creation_time = Timestamp::from(100);
    let test_time = Timestamp::from(120);
    let ttl = TimeDiff::from(Duration::from_millis(100));

    let mut rng = crate::new_rng();
    let deploy1 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy2 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy3 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy4 = generate_deploy(
        &mut rng,
        creation_time + Duration::from_secs(20).into(),
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let mut proposer = BlockProposerReady::default();

    // pending
    proposer.add_deploy(creation_time, Box::new(deploy1.clone()));
    proposer.add_deploy(creation_time, Box::new(deploy2));
    proposer.add_deploy(creation_time, Box::new(deploy3));
    proposer.add_deploy(creation_time, Box::new(deploy4));

    // pending => finalized
    proposer.finalized_deploys(vec![deploy1.deploy_or_transfer_hash()]);

    assert_eq!(proposer.sets.pending_deploys.len(), 3);
    assert!(proposer.sets.finalized_deploys.contains_key(deploy1.id()));

    // test for retained values
    let pruned = proposer.prune(test_time);
    assert_eq!(pruned, 0);

    assert_eq!(proposer.sets.pending_deploys.len(), 3);
    assert_eq!(proposer.sets.finalized_deploys.len(), 1);
    assert!(proposer.sets.finalized_deploys.contains_key(&deploy1.id()));

    // now move the clock to make some things expire
    let pruned = proposer.prune(expired_time);
    assert_eq!(pruned, 3);

    assert_eq!(proposer.sets.pending_deploys.len(), 1); // deploy4 is still valid
    assert_eq!(proposer.sets.finalized_deploys.len(), 0);
}

#[test]
fn should_keep_track_of_unhandled_deploys() {
    let creation_time = Timestamp::from(100);
    let test_time = Timestamp::from(120);
    let ttl = TimeDiff::from(Duration::from_millis(100));

    let mut rng = crate::new_rng();
    let deploy1 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let deploy2 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    let mut proposer = BlockProposerReady::default();

    // We do NOT add deploy2...
    proposer.add_deploy(creation_time, Box::new(deploy1.clone()));
    // But we DO mark it as finalized, by it's hash
    proposer.finalized_deploys(vec![
        deploy1.deploy_or_transfer_hash(),
        deploy2.deploy_or_transfer_hash(),
    ]);

    assert!(
        proposer.contains_finalized(deploy1.id()),
        "should contain deploy1"
    );
    assert!(
        proposer.contains_finalized(deploy2.id()),
        "deploy2's hash should be considered seen"
    );
    assert!(
        !proposer.sets.finalized_deploys.contains_key(deploy2.id()),
        "should not yet contain deploy2"
    );
    assert!(
        proposer.contains_finalized(deploy2.id()),
        "should recognize deploy2 as finalized"
    );

    assert!(
        deploy2
            .header()
            .is_valid(&proposer.deploy_config, test_time),
        "deploy2 should be valid"
    );

    // Now we add Deploy2
    proposer.add_deploy(creation_time, Box::new(deploy2.clone()));
    assert!(
        proposer.sets.finalized_deploys.contains_key(deploy2.id()),
        "deploy2 should now be in finalized_deploys"
    );
    assert!(
        !proposer.unhandled_finalized.contains(deploy2.id()),
        "deploy2 should not be in unhandled_finalized"
    );
}

#[test]
fn should_respect_limits_for_wasmless_transfer_hashes() {
    test_proposer_with(TestArgs {
        transfer_count: 30,
        max_transfer_count: 20,
        proposed_count: 20,
        remaining_pending_count: 10,
        ..Default::default()
    });
}

#[test]
fn should_respect_limits_for_deploy_hashes() {
    test_proposer_with(TestArgs {
        deploy_count: 30,
        max_deploy_count: 20,
        proposed_count: 20,
        remaining_pending_count: 10,
        ..Default::default()
    });
}

#[test]
fn should_respect_limits_for_deploys_and_transfers_together() {
    test_proposer_with(TestArgs {
        transfer_count: 30,
        max_transfer_count: 20,
        deploy_count: 30,
        max_deploy_count: 20,
        proposed_count: 40,
        remaining_pending_count: 20,
        ..Default::default()
    });
}

#[test]
fn should_respect_limits_for_gas_cost() {
    test_proposer_with(TestArgs {
        transfer_count: 15,
        max_transfer_count: 20,
        deploy_count: 30,
        max_deploy_count: 20,
        payment_amount: default_gas_payment(),
        block_gas_limit: 10,
        proposed_count: 25,
        remaining_pending_count: 20,
        ..Default::default()
    });
}

#[test]
fn should_respect_block_gas_limit_for_deploys() {
    test_proposer_with(TestArgs {
        deploy_count: 15,
        payment_amount: default_gas_payment(),
        block_gas_limit: 5,
        max_deploy_count: 15,
        proposed_count: 5,
        remaining_pending_count: 10,
        ..Default::default()
    });
}

#[test]
fn should_propose_deploy_if_block_size_limit_met() {
    test_proposer_with(TestArgs {
        transfer_count: 1,
        deploy_count: 1,
        payment_amount: default_gas_payment(),
        block_gas_limit: 10,
        max_transfer_count: 2,
        max_deploy_count: 2,
        proposed_count: 2,
        remaining_pending_count: 0,
        max_block_size: Some(2 * DEPLOY_APPROX_MIN_SIZE),
    });
}

#[test]
fn should_not_propose_deploy_if_block_size_limit_within_threshold() {
    test_proposer_with(TestArgs {
        transfer_count: 2,
        deploy_count: 2,
        payment_amount: default_gas_payment(),
        block_gas_limit: 10,
        max_transfer_count: 3,
        max_deploy_count: 3,
        proposed_count: 4,
        remaining_pending_count: 0,
        max_block_size: Some(2 * DEPLOY_APPROX_MIN_SIZE),
    });
}

#[test]
fn should_not_propose_deploy_if_block_size_limit_passed() {
    test_proposer_with(TestArgs {
        deploy_count: 3,
        transfer_count: 2, // transfers should -not- count towards the block size limit
        payment_amount: default_gas_payment(),
        block_gas_limit: 100,
        max_transfer_count: 5,
        max_deploy_count: 5,
        proposed_count: 4,
        remaining_pending_count: 1,
        max_block_size: Some(2 * DEPLOY_APPROX_MIN_SIZE),
    });
}

#[test]
fn should_allow_transfers_to_exceed_block_size_limit() {
    test_proposer_with(TestArgs {
        deploy_count: 3,
        transfer_count: 60,
        payment_amount: default_gas_payment(),
        block_gas_limit: 100,
        max_transfer_count: 40,
        max_deploy_count: 5,
        proposed_count: 42,
        remaining_pending_count: 21,
        max_block_size: Some(2 * DEPLOY_APPROX_MIN_SIZE),
    });
}

#[derive(Default)]
struct TestArgs {
    /// Number of deploys to create.
    deploy_count: u32,
    /// Max deploys to propose.
    max_deploy_count: u32,
    /// Number of transfer deploys to create.
    transfer_count: u32,
    /// Number of transfer deploys to create.
    max_transfer_count: u32,
    /// Payment amount for transfers.
    payment_amount: Gas,
    /// Max gas cost for block.
    block_gas_limit: u64,
    /// Post-finalization of proposed block, how many transfers and deploys remain.
    remaining_pending_count: usize,
    /// Block deploy count proposed.
    proposed_count: usize,
    /// Block size limit in bytes.
    max_block_size: Option<usize>,
}

/// Test the block_proposer by generating deploys and transfers with variable limits, asserting
/// on internal counts post-finalization.
fn test_proposer_with(
    TestArgs {
        deploy_count,
        max_deploy_count,
        transfer_count,
        max_transfer_count,
        payment_amount,
        block_gas_limit,
        remaining_pending_count,
        proposed_count,
        max_block_size,
    }: TestArgs,
) -> BlockProposerReady {
    let creation_time = Timestamp::from(100);
    let test_time = Timestamp::from(120);
    let ttl = TimeDiff::from(Duration::from_millis(100));

    let mut rng = crate::new_rng();
    let mut proposer = BlockProposerReady::default();
    let mut config = proposer.deploy_config;
    // defaults are 10, 1000 respectively
    config.block_max_deploy_count = max_deploy_count;
    config.block_max_transfer_count = max_transfer_count;
    config.block_gas_limit = block_gas_limit;
    if let Some(max_block_size) = max_block_size {
        config.max_block_size = max_block_size as u32;
    }

    for _ in 0..deploy_count {
        let deploy = generate_deploy(
            &mut rng,
            creation_time,
            ttl,
            vec![],
            payment_amount,
            DEFAULT_TEST_GAS_PRICE,
        );
        proposer.add_deploy(creation_time, Box::new(deploy));
    }
    for _ in 0..transfer_count {
        let transfer = generate_transfer(&mut rng, creation_time, ttl, vec![], payment_amount);
        proposer.add_deploy(creation_time, Box::new(transfer));
    }

    let block =
        proposer.propose_block_payload(config, BlockContext::new(test_time, vec![]), vec![], true);
    let all_deploys = block.deploys_and_transfers_iter().collect_vec();
    proposer.finalized_deploys(all_deploys.iter().copied());
    assert_eq!(
        all_deploys.len(),
        proposed_count,
        "should have a proposed_count of {}, but got {}",
        proposed_count,
        all_deploys.len()
    );
    assert_eq!(
        proposer.sets.pending_deploys.len() + proposer.sets.pending_transfers.len(),
        remaining_pending_count,
        "should have a remaining_pending_count of {}, but got {}",
        remaining_pending_count,
        proposer.sets.pending_deploys.len() + proposer.sets.pending_transfers.len()
    );
    proposer
}

#[test]
fn should_return_deploy_dependencies() {
    let creation_time = Timestamp::from(100);
    let ttl = TimeDiff::from(Duration::from_millis(100));
    let block_time = Timestamp::from(120);

    let mut rng = crate::new_rng();
    let deploy1 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );
    // let deploy2 depend on deploy1
    let deploy2 = generate_deploy(
        &mut rng,
        creation_time,
        ttl,
        vec![*deploy1.id()],
        default_gas_payment(),
        DEFAULT_TEST_GAS_PRICE,
    );

    let mut proposer = BlockProposerReady::default();

    // add deploy2
    proposer.add_deploy(creation_time, Box::new(deploy2.clone()));

    // deploy2 has an unsatisfied dependency
    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time, vec![]),
        vec![],
        true,
    );
    assert!(block.deploy_hashes().is_empty());
    assert!(block.transfer_hashes().is_empty());

    // add deploy1
    proposer.add_deploy(creation_time, Box::new(deploy1.clone()));

    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time, vec![]),
        vec![],
        true,
    );
    let deploys: Vec<DeployOrTransferHash> = block.deploys_and_transfers_iter().collect();
    // only deploy1 should be returned, as it has no dependencies
    assert_eq!(deploys.len(), 1);
    assert!(deploys.contains(&deploy1.deploy_or_transfer_hash()));

    // the deploy will be included in block 1
    proposer.finalized_deploys(deploys.iter().copied());

    let block = proposer.propose_block_payload(
        DeployConfig::default(),
        BlockContext::new(block_time, vec![]),
        vec![],
        true,
    );
    // `blocks` contains a block that contains deploy1 now, so we should get deploy2
    let deploys2 = block.deploy_hashes();
    assert_eq!(deploys2.len(), 1);
    assert!(deploys2.contains(deploy2.id()));
}
