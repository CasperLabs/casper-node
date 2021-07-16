//! Unit tests for the storage component.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    convert::TryFrom,
};

use lmdb::{Cursor, Transaction};
use rand::{prelude::SliceRandom, Rng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use smallvec::smallvec;

use casper_types::{EraId, ExecutionResult, ProtocolVersion, PublicKey, SecretKey};

use super::{
    construct_block_body_to_block_header_reverse_lookup, garbage_collect_block_body_v2_db, Config,
    Storage,
};
use crate::{
    components::storage::lmdb_ext::WriteTransactionExt,
    crypto::{hash::Digest, AsymmetricKeyExt},
    effect::{
        requests::{StateStoreRequest, StorageRequest},
        Multiple,
    },
    testing::{ComponentHarness, TestRng, UnitTestEvent},
    types::{
        Block, BlockHash, BlockHeader, BlockSignatures, Deploy, DeployHash, DeployMetadata,
        FinalitySignature,
    },
    utils::WithDir,
};

fn new_config(harness: &ComponentHarness<UnitTestEvent>) -> Config {
    const MIB: usize = 1024 * 1024;

    // Restrict all stores to 50 mibibytes, to catch issues before filling up the entire disk.
    Config {
        path: harness.tmp.path().join("storage"),
        max_block_store_size: 50 * MIB,
        max_deploy_store_size: 50 * MIB,
        max_deploy_metadata_store_size: 50 * MIB,
        max_state_store_size: 50 * MIB,
        enable_mem_deduplication: false,
    }
}

/// Storage component test fixture.
///
/// Creates a storage component in a temporary directory.
///
/// # Panics
///
/// Panics if setting up the storage fixture fails.
fn storage_fixture(harness: &ComponentHarness<UnitTestEvent>) -> Storage {
    let cfg = new_config(harness);
    Storage::new(
        &WithDir::new(harness.tmp.path(), cfg),
        None,
        ProtocolVersion::from_parts(1, 0, 0),
        false,
    )
    .expect("could not create storage component fixture")
}

/// Storage component test fixture.
///
/// Creates a storage component in a temporary directory, but with a hard reset to a specified era.
///
/// # Panics
///
/// Panics if setting up the storage fixture fails.
fn storage_fixture_with_hard_reset(
    harness: &ComponentHarness<UnitTestEvent>,
    reset_era_id: EraId,
) -> Storage {
    let cfg = new_config(harness);
    Storage::new(
        &WithDir::new(harness.tmp.path(), cfg),
        Some(reset_era_id),
        ProtocolVersion::from_parts(1, 1, 0),
        false,
    )
    .expect("could not create storage component fixture")
}

/// Storage component test fixture.
///
/// Creates a storage component in a temporary directory, but with a hard reset to a specified era.
///
/// # Panics
///
/// Panics if setting up the storage fixture fails.
fn storage_fixture_with_hard_reset_and_protocol_version(
    harness: &ComponentHarness<UnitTestEvent>,
    reset_era_id: EraId,
    protocol_version: ProtocolVersion,
) -> Storage {
    let cfg = new_config(harness);
    Storage::new(
        &WithDir::new(harness.tmp.path(), cfg),
        Some(reset_era_id),
        protocol_version,
        false,
    )
    .expect("could not create storage component fixture")
}

/// Creates a random block with a specific block height.
fn random_block_at_height(rng: &mut TestRng, height: u64) -> Box<Block> {
    let mut block = Box::new(Block::random(rng));
    block.set_height(height);
    block
}

/// Creates 3 random signatures for the given block.
fn random_signatures(rng: &mut TestRng, block: &Block) -> BlockSignatures {
    let block_hash = *block.hash();
    let era_id = block.header().era_id();
    let mut block_signatures = BlockSignatures::new(block_hash, era_id);
    for _ in 0..3 {
        let secret_key = SecretKey::random(rng);
        let signature = FinalitySignature::new(
            block_hash,
            era_id,
            &secret_key,
            PublicKey::from(&secret_key),
        );
        block_signatures.insert_proof(signature.public_key, signature.signature);
    }
    block_signatures
}

/// Requests block header at a specific height from a storage component.
fn get_block_header_at_height(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    height: u64,
) -> Option<BlockHeader> {
    let response = harness.send_request(storage, |responder| {
        StorageRequest::GetBlockHeaderAtHeight { height, responder }.into()
    });
    assert!(harness.is_idle());
    response
}

/// Requests block at a specific height from a storage component.
fn get_block_at_height(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    height: u64,
) -> Option<Block> {
    let response = harness.send_request(storage, |responder| {
        StorageRequest::GetBlockAtHeight { height, responder }.into()
    });
    assert!(harness.is_idle());
    response
}

/// Loads a block from a storage component.
fn get_block(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    block_hash: BlockHash,
) -> Option<Block> {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::GetBlock {
            block_hash,
            responder,
        }
        .into()
    });
    assert!(harness.is_idle());
    response
}

/// Loads a block's signatures from a storage component.
fn get_block_signatures(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    block_hash: BlockHash,
) -> Option<BlockSignatures> {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::GetBlockSignatures {
            block_hash,
            responder,
        }
        .into()
    });
    assert!(harness.is_idle());
    response
}

/// Loads a set of deploys from a storage component.
fn get_deploys(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    deploy_hashes: Multiple<DeployHash>,
) -> Vec<Option<Deploy>> {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::GetDeploys {
            deploy_hashes: deploy_hashes.to_vec(),
            responder,
        }
        .into()
    });
    assert!(harness.is_idle());
    response
}

/// Loads a deploy with associated metadata from the storage component.
fn get_deploy_and_metadata(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    deploy_hash: DeployHash,
) -> Option<(Deploy, DeployMetadata)> {
    let response = harness.send_request(storage, |responder| {
        StorageRequest::GetDeployAndMetadata {
            deploy_hash,
            responder,
        }
        .into()
    });
    assert!(harness.is_idle());
    response
}

/// Requests the highest block from a storage component.
fn get_highest_block(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
) -> Option<Block> {
    let response = harness.send_request(storage, |responder| {
        StorageRequest::GetHighestBlock { responder }.into()
    });
    assert!(harness.is_idle());
    response
}

/// Loads state from the storage component.
fn load_state<T>(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    key: Cow<'static, [u8]>,
) -> Option<T>
where
    T: DeserializeOwned,
{
    let response: Option<Vec<u8>> = harness.send_request(storage, move |responder| {
        StateStoreRequest::Load { key, responder }.into()
    });
    assert!(harness.is_idle());

    // NOTE: Unfortunately, the deserialization logic is duplicated here from the effect builder.
    response.map(|raw| bincode::deserialize(&raw).expect("deserialization failed"))
}

/// Stores a block in a storage component.
fn put_block(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    block: Box<Block>,
) -> bool {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::PutBlock { block, responder }.into()
    });
    assert!(harness.is_idle());
    response
}

/// Stores a block's signatures in a storage component.
fn put_block_signatures(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    signatures: BlockSignatures,
) -> bool {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::PutBlockSignatures {
            signatures,
            responder,
        }
        .into()
    });
    assert!(harness.is_idle());
    response
}

/// Stores a deploy in a storage component.
fn put_deploy(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    deploy: Box<Deploy>,
) -> bool {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::PutDeploy { deploy, responder }.into()
    });
    assert!(harness.is_idle());
    response
}

/// Stores execution results in a storage component.
fn put_execution_results(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    block_hash: BlockHash,
    execution_results: HashMap<DeployHash, ExecutionResult>,
) {
    let response = harness.send_request(storage, move |responder| {
        StorageRequest::PutExecutionResults {
            block_hash: Box::new(block_hash),
            execution_results,
            responder,
        }
        .into()
    });
    assert!(harness.is_idle());
    response
}

/// Saves state from the storage component.
fn save_state<T>(
    harness: &mut ComponentHarness<UnitTestEvent>,
    storage: &mut Storage,
    key: Cow<'static, [u8]>,
    value: &T,
) where
    T: Serialize,
{
    // NOTE: Unfortunately, the serialization logic is duplicated here from the effect builder.
    let data = bincode::serialize(value).expect("serialization failed");
    harness.send_request(storage, move |responder| {
        StateStoreRequest::Save {
            key,
            responder,
            data,
        }
        .into()
    });
    assert!(harness.is_idle());
}

#[test]
fn get_block_of_non_existing_block_returns_none() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let block_hash = BlockHash::random(&mut harness.rng);
    let response = get_block(&mut harness, &mut storage, block_hash);

    assert!(response.is_none());
    assert!(harness.is_idle());
}

#[test]
fn can_put_and_get_block() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create a random block, store and load it.
    let block = Box::new(Block::random(&mut harness.rng));

    let was_new = put_block(&mut harness, &mut storage, block.clone());
    assert!(was_new, "putting block should have returned `true`");

    // Storing the same block again should work, but yield a result of `true`.
    let was_new_second_time = put_block(&mut harness, &mut storage, block.clone());
    assert!(
        was_new_second_time,
        "storing block the second time should have returned `true`"
    );

    let response = get_block(&mut harness, &mut storage, *block.hash());
    assert_eq!(response.as_ref(), Some(&*block));

    // Also ensure we can retrieve just the header.
    let response = harness.send_request(&mut storage, |responder| {
        StorageRequest::GetBlockHeader {
            block_hash: *block.hash(),
            responder,
        }
        .into()
    });

    assert_eq!(response.as_ref(), Some(block.header()));
}

#[test]
fn test_get_block_header_and_finality_signatures_by_height() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create a random block, store and load it.
    let block = Block::random(&mut harness.rng);
    let mut block_signatures = BlockSignatures::new(block.header().hash(), block.header().era_id());

    {
        let alice_secret_key =
            SecretKey::ed25519_from_bytes([1; SecretKey::ED25519_LENGTH]).unwrap();
        let FinalitySignature {
            public_key,
            signature,
            ..
        } = FinalitySignature::new(
            block.header().hash(),
            block.header().era_id(),
            &alice_secret_key,
            PublicKey::from(&alice_secret_key),
        );
        block_signatures.insert_proof(public_key, signature);
    }

    {
        let bob_secret_key = SecretKey::ed25519_from_bytes([2; SecretKey::ED25519_LENGTH]).unwrap();
        let FinalitySignature {
            public_key,
            signature,
            ..
        } = FinalitySignature::new(
            block.header().hash(),
            block.header().era_id(),
            &bob_secret_key,
            PublicKey::from(&bob_secret_key),
        );
        block_signatures.insert_proof(public_key, signature);
    }

    let was_new = put_block(&mut harness, &mut storage, Box::new(block.clone()));
    assert!(was_new, "putting block should have returned `true`");

    let mut txn = storage
        .env
        .begin_rw_txn()
        .expect("Could not start transaction");
    let was_new = txn
        .put_value(
            storage.block_metadata_db,
            &block.hash(),
            &block_signatures,
            true,
        )
        .expect("should put value into LMDB");
    assert!(
        was_new,
        "putting block signatures should have returned `true`"
    );
    txn.commit().expect("Could not commit transaction");

    {
        let block_header = storage
            .get_block_header_by_hash(block.hash())
            .expect("should not throw exception")
            .expect("should not be None");
        assert_eq!(
            block_header,
            block.header().clone(),
            "Should have retrieved expected block header"
        );
    }

    {
        let block_header_with_metadata = storage
            .read_block_header_and_finality_signatures_by_height(block.header().height())
            .expect("should not throw exception")
            .expect("should not be None");
        assert_eq!(
            block_header_with_metadata.block_header,
            block.header().clone(),
            "Should have retrieved expected block header"
        );
        assert_eq!(
            block_header_with_metadata.block_signatures, block_signatures,
            "Should have retrieved expected block signatures"
        );
    }
}

#[test]
fn can_retrieve_block_by_height() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create a random block, load and store it.
    let block_33 = Box::new(Block::random_with_specifics(
        &mut harness.rng,
        EraId::new(1),
        33,
        ProtocolVersion::V1_0_0,
        true,
    ));
    let block_14 = Box::new(Block::random_with_specifics(
        &mut harness.rng,
        EraId::new(1),
        14,
        ProtocolVersion::V1_0_0,
        false,
    ));
    let block_99 = Box::new(Block::random_with_specifics(
        &mut harness.rng,
        EraId::new(2),
        99,
        ProtocolVersion::V1_0_0,
        true,
    ));

    // Both block at ID and highest block should return `None` initially.
    assert!(get_block_at_height(&mut harness, &mut storage, 0).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 0).is_none());
    assert!(get_highest_block(&mut harness, &mut storage).is_none());
    assert!(get_block_at_height(&mut harness, &mut storage, 14).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 14).is_none());
    assert!(get_block_at_height(&mut harness, &mut storage, 33).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 33).is_none());
    assert!(get_block_at_height(&mut harness, &mut storage, 99).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 99).is_none());

    // Inserting 33 changes this.
    let was_new = put_block(&mut harness, &mut storage, block_33.clone());
    assert!(was_new);

    assert_eq!(
        get_highest_block(&mut harness, &mut storage).as_ref(),
        Some(&*block_33)
    );
    assert!(get_block_at_height(&mut harness, &mut storage, 0).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 0).is_none());
    assert!(get_block_at_height(&mut harness, &mut storage, 14).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 14).is_none());
    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 33).as_ref(),
        Some(&*block_33)
    );
    assert_eq!(
        get_block_header_at_height(&mut harness, &mut storage, 33).as_ref(),
        Some(block_33.header())
    );
    assert!(get_block_at_height(&mut harness, &mut storage, 99).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 99).is_none());

    // Inserting block with height 14, no change in highest.
    let was_new = put_block(&mut harness, &mut storage, block_14.clone());
    assert!(was_new);

    assert_eq!(
        get_highest_block(&mut harness, &mut storage).as_ref(),
        Some(&*block_33)
    );
    assert!(get_block_at_height(&mut harness, &mut storage, 0).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 0).is_none());
    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 14).as_ref(),
        Some(&*block_14)
    );
    assert_eq!(
        get_block_header_at_height(&mut harness, &mut storage, 14).as_ref(),
        Some(block_14.header())
    );
    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 33).as_ref(),
        Some(&*block_33)
    );
    assert_eq!(
        get_block_header_at_height(&mut harness, &mut storage, 33).as_ref(),
        Some(block_33.header())
    );
    assert!(get_block_at_height(&mut harness, &mut storage, 99).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 99).is_none());

    // Inserting block with height 99, changes highest.
    let was_new = put_block(&mut harness, &mut storage, block_99.clone());
    assert!(was_new);

    assert_eq!(
        get_highest_block(&mut harness, &mut storage).as_ref(),
        Some(&*block_99)
    );
    assert!(get_block_at_height(&mut harness, &mut storage, 0).is_none());
    assert!(get_block_header_at_height(&mut harness, &mut storage, 0).is_none());
    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 14).as_ref(),
        Some(&*block_14)
    );
    assert_eq!(
        get_block_header_at_height(&mut harness, &mut storage, 14).as_ref(),
        Some(block_14.header())
    );
    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 33).as_ref(),
        Some(&*block_33)
    );
    assert_eq!(
        get_block_header_at_height(&mut harness, &mut storage, 33).as_ref(),
        Some(block_33.header())
    );
    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 99).as_ref(),
        Some(&*block_99)
    );
    assert_eq!(
        get_block_header_at_height(&mut harness, &mut storage, 99).as_ref(),
        Some(block_99.header())
    );
}

#[test]
#[should_panic(expected = "duplicate entries")]
fn different_block_at_height_is_fatal() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create two different blocks at the same height.
    let block_44_a = Box::new(Block::random_with_specifics(
        &mut harness.rng,
        EraId::new(1),
        44,
        ProtocolVersion::V1_0_0,
        false,
    ));
    let block_44_b = Box::new(Block::random_with_specifics(
        &mut harness.rng,
        EraId::new(1),
        44,
        ProtocolVersion::V1_0_0,
        false,
    ));

    let was_new = put_block(&mut harness, &mut storage, block_44_a.clone());
    assert!(was_new);

    let was_new = put_block(&mut harness, &mut storage, block_44_a);
    assert!(was_new);

    // Putting a different block with the same height should now crash.
    put_block(&mut harness, &mut storage, block_44_b);
}

#[test]
fn get_vec_of_non_existing_deploy_returns_nones() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let deploy_id = DeployHash::random(&mut harness.rng);
    let response = get_deploys(&mut harness, &mut storage, smallvec![deploy_id]);
    assert_eq!(response, vec![None]);

    // Also verify that we can retrieve using an empty set of deploy hashes.
    let response = get_deploys(&mut harness, &mut storage, smallvec![]);
    assert!(response.is_empty());
}

#[test]
fn can_retrieve_store_and_load_deploys() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create a random deploy, store and load it.
    let deploy = Box::new(Deploy::random(&mut harness.rng));

    let was_new = put_deploy(&mut harness, &mut storage, deploy.clone());
    assert!(was_new, "putting deploy should have returned `true`");

    // Storing the same deploy again should work, but yield a result of `false`.
    let was_new_second_time = put_deploy(&mut harness, &mut storage, deploy.clone());
    assert!(
        !was_new_second_time,
        "storing deploy the second time should have returned `false`"
    );

    // Retrieve the stored deploy.
    let response = get_deploys(&mut harness, &mut storage, smallvec![*deploy.id()]);
    assert_eq!(response, vec![Some(deploy.as_ref().clone())]);

    // Also ensure we can retrieve just the header.
    let response = harness.send_request(&mut storage, |responder| {
        StorageRequest::GetDeployHeaders {
            deploy_hashes: vec![*deploy.id()],
            responder,
        }
        .into()
    });
    assert_eq!(response, vec![Some(deploy.header().clone())]);

    // Finally try to get the metadata as well. Since we did not store any, we expect empty default
    // metadata to present.
    let (deploy_response, metadata_response) = harness
        .send_request(&mut storage, |responder| {
            StorageRequest::GetDeployAndMetadata {
                deploy_hash: *deploy.id(),
                responder,
            }
            .into()
        })
        .expect("no deploy with metadata returned");

    assert_eq!(deploy_response, *deploy);
    assert_eq!(metadata_response, DeployMetadata::default());
}

#[test]
fn storing_and_loading_a_lot_of_deploys_does_not_exhaust_handles() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let total = 1000;
    let batch_size = 25;

    let mut deploy_hashes = Vec::new();

    for _ in 0..total {
        let deploy = Box::new(Deploy::random(&mut harness.rng));
        deploy_hashes.push(*deploy.id());
        put_deploy(&mut harness, &mut storage, deploy);
    }

    // Shuffle deploy hashes around to get a random order.
    deploy_hashes.as_mut_slice().shuffle(&mut harness.rng);

    // Retrieve all from storage, ensuring they are found.
    for chunk in deploy_hashes.chunks(batch_size) {
        let result = get_deploys(&mut harness, &mut storage, chunk.iter().cloned().collect());
        assert!(result.iter().all(Option::is_some));
    }
}

#[test]
fn store_execution_results_for_two_blocks() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let deploy = Deploy::random(&mut harness.rng);

    let block_hash_a = BlockHash::random(&mut harness.rng);
    let block_hash_b = BlockHash::random(&mut harness.rng);

    // Store the deploy.
    put_deploy(&mut harness, &mut storage, Box::new(deploy.clone()));

    // Ensure deploy exists.
    assert_eq!(
        get_deploys(&mut harness, &mut storage, smallvec![*deploy.id()]),
        vec![Some(deploy.clone())]
    );

    // Put first execution result.
    let first_result: ExecutionResult = harness.rng.gen();
    let mut first_results = HashMap::new();
    first_results.insert(*deploy.id(), first_result.clone());
    put_execution_results(&mut harness, &mut storage, block_hash_a, first_results);

    // Retrieve and check if correct.
    let (first_deploy, first_metadata) =
        get_deploy_and_metadata(&mut harness, &mut storage, *deploy.id())
            .expect("missing on first attempt");
    assert_eq!(first_deploy, deploy);
    let mut expected_per_block_results = HashMap::new();
    expected_per_block_results.insert(block_hash_a, first_result);
    assert_eq!(first_metadata.execution_results, expected_per_block_results);

    // Add second result for the same deploy, different block.
    let second_result: ExecutionResult = harness.rng.gen();
    let mut second_results = HashMap::new();
    second_results.insert(*deploy.id(), second_result.clone());
    put_execution_results(&mut harness, &mut storage, block_hash_b, second_results);

    // Retrieve the deploy again, should now contain both.
    let (second_deploy, second_metadata) =
        get_deploy_and_metadata(&mut harness, &mut storage, *deploy.id())
            .expect("missing on second attempt");
    assert_eq!(second_deploy, deploy);
    expected_per_block_results.insert(block_hash_b, second_result);
    assert_eq!(
        second_metadata.execution_results,
        expected_per_block_results
    );
}

#[test]
fn store_random_execution_results() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // We store results for two different blocks. Each block will have five deploys executed in it,
    // with two of these deploys being shared by both blocks, while the remaining three are unique
    // per block.
    let block_hash_a = BlockHash::random(&mut harness.rng);
    let block_hash_b = BlockHash::random(&mut harness.rng);

    // Create the shared deploys.
    let shared_deploys = vec![
        Deploy::random(&mut harness.rng),
        Deploy::random(&mut harness.rng),
    ];

    // Store shared deploys.
    for deploy in &shared_deploys {
        put_deploy(&mut harness, &mut storage, Box::new(deploy.clone()));
    }

    // We collect the expected result per deploy in parallel to adding them.
    let mut expected_outcome = HashMap::new();

    fn setup_block(
        harness: &mut ComponentHarness<UnitTestEvent>,
        storage: &mut Storage,
        expected_outcome: &mut HashMap<DeployHash, HashMap<BlockHash, ExecutionResult>>,
        block_hash: &BlockHash,
        shared_deploys: &[Deploy],
    ) {
        let unique_count = 3;

        // Results for a single block.
        let mut block_results = HashMap::new();

        // Add three unique deploys to block.
        for _ in 0..unique_count {
            let deploy = Deploy::random(&mut harness.rng);

            // Store unique deploy.
            put_deploy(harness, storage, Box::new(deploy.clone()));

            let execution_result: ExecutionResult = harness.rng.gen();

            // Insert deploy results for the unique block-deploy combination.
            let mut map = HashMap::new();
            map.insert(*block_hash, execution_result.clone());
            expected_outcome.insert(*deploy.id(), map);

            // Add to our expected outcome.
            block_results.insert(*deploy.id(), execution_result);
        }

        // Insert the shared deploys as well.
        for shared_deploy in shared_deploys {
            let execution_result: ExecutionResult = harness.rng.gen();

            // Insert the new result and ensure it is not present yet.
            let result = block_results.insert(*shared_deploy.id(), execution_result.clone());
            assert!(result.is_none());

            // Insert into expected outcome.
            let deploy_expected = expected_outcome.entry(*shared_deploy.id()).or_default();
            let prev = deploy_expected.insert(*block_hash, execution_result.clone());
            // Ensure we are not replacing something.
            assert!(prev.is_none());
        }

        // We should have all results for our block collected for the input.
        assert_eq!(block_results.len(), unique_count + shared_deploys.len());

        // Now we can submit the block's execution results.
        put_execution_results(harness, storage, *block_hash, block_results);
    }

    setup_block(
        &mut harness,
        &mut storage,
        &mut expected_outcome,
        &block_hash_a,
        &shared_deploys,
    );

    setup_block(
        &mut harness,
        &mut storage,
        &mut expected_outcome,
        &block_hash_b,
        &shared_deploys,
    );

    // At this point, we are all set up and ready to receive results. Iterate over every deploy and
    // see if its execution-data-per-block matches our expectations.
    for (deploy_hash, raw_meta) in expected_outcome.iter() {
        let (deploy, metadata) = get_deploy_and_metadata(&mut harness, &mut storage, *deploy_hash)
            .expect("missing deploy");

        assert_eq!(deploy_hash, deploy.id());

        assert_eq!(raw_meta, &metadata.execution_results);
    }
}

#[test]
fn store_execution_results_twice_for_same_block_deploy_pair() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let block_hash = BlockHash::random(&mut harness.rng);
    let deploy_hash = DeployHash::random(&mut harness.rng);

    let mut exec_result_1 = HashMap::new();
    exec_result_1.insert(deploy_hash, harness.rng.gen());

    let mut exec_result_2 = HashMap::new();
    exec_result_2.insert(deploy_hash, harness.rng.gen());

    put_execution_results(&mut harness, &mut storage, block_hash, exec_result_1);

    // Storing a second execution result for the same deploy on the same block should panic.
    put_execution_results(&mut harness, &mut storage, block_hash, exec_result_2);
}

#[test]
fn store_identical_execution_results() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let block_hash = BlockHash::random(&mut harness.rng);
    let deploy_hash = DeployHash::random(&mut harness.rng);

    let mut exec_result = HashMap::new();
    exec_result.insert(deploy_hash, harness.rng.gen());

    put_execution_results(&mut harness, &mut storage, block_hash, exec_result.clone());

    // We should be fine storing the exact same result twice.
    put_execution_results(&mut harness, &mut storage, block_hash, exec_result);
}

/// Example state used in storage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StateData {
    a: Vec<u32>,
    b: i32,
}

#[test]
fn store_and_load_state_data() {
    let key1 = b"sample-key-1".to_vec();
    let key2 = b"exkey-2".to_vec();

    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Initially, both keys should return nothing.
    let load1 = load_state::<StateData>(&mut harness, &mut storage, key1.clone().into());
    let load2 = load_state::<StateData>(&mut harness, &mut storage, key2.clone().into());

    assert!(load1.is_none());
    assert!(load2.is_none());

    let data1 = StateData { a: vec![1], b: -1 };
    let data2 = StateData { a: vec![], b: 2 };

    // Store one after another.
    save_state(&mut harness, &mut storage, key1.clone().into(), &data1);
    let load1 = load_state::<StateData>(&mut harness, &mut storage, key1.clone().into());
    let load2 = load_state::<StateData>(&mut harness, &mut storage, key2.clone().into());

    assert_eq!(load1, Some(data1.clone()));
    assert!(load2.is_none());

    save_state(&mut harness, &mut storage, key2.clone().into(), &data2);
    let load1 = load_state::<StateData>(&mut harness, &mut storage, key1.clone().into());
    let load2 = load_state::<StateData>(&mut harness, &mut storage, key2.clone().into());

    assert_eq!(load1, Some(data1));
    assert_eq!(load2, Some(data2.clone()));

    // Overwrite `data1` in store.
    save_state(&mut harness, &mut storage, key1.clone().into(), &data2);
    let load1 = load_state::<StateData>(&mut harness, &mut storage, key1.into());
    let load2 = load_state::<StateData>(&mut harness, &mut storage, key2.into());

    assert_eq!(load1, Some(data2.clone()));
    assert_eq!(load2, Some(data2));
}

#[test]
fn persist_state_data() {
    let key = b"sample-key-1".to_vec();

    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let load = load_state::<StateData>(&mut harness, &mut storage, key.clone().into());
    assert!(load.is_none());

    let data = StateData {
        a: vec![1, 2, 3, 4, 5, 6],
        b: -1,
    };

    // Store one after another.
    save_state(&mut harness, &mut storage, key.clone().into(), &data);
    let load = load_state::<StateData>(&mut harness, &mut storage, key.clone().into());
    assert_eq!(load, Some(data.clone()));

    let (on_disk, rng) = harness.into_parts();
    let mut harness = ComponentHarness::builder()
        .on_disk(on_disk)
        .rng(rng)
        .build();
    let mut storage = storage_fixture(&harness);

    let load = load_state::<StateData>(&mut harness, &mut storage, key.into());
    assert_eq!(load, Some(data));
}

#[test]
fn test_legacy_interface() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    let deploy = Box::new(Deploy::random(&mut harness.rng));
    let was_new = put_deploy(&mut harness, &mut storage, deploy.clone());
    assert!(was_new);

    // Ensure we get the deploy we expect.
    let result = storage.get_deploy(*deploy.id()).expect("should get deploy");
    assert_eq!(result, Some(*deploy));

    // A non-existent deploy should simply return `None`.
    assert!(storage
        .get_deploy(DeployHash::random(&mut harness.rng))
        .expect("should get deploy")
        .is_none())
}

#[test]
fn persist_blocks_deploys_and_deploy_metadata_across_instantiations() {
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create some sample data.
    let deploy = Deploy::random(&mut harness.rng);
    let block = random_block_at_height(&mut harness.rng, 42);
    let execution_result: ExecutionResult = harness.rng.gen();

    put_deploy(&mut harness, &mut storage, Box::new(deploy.clone()));
    put_block(&mut harness, &mut storage, block.clone());
    let mut execution_results = HashMap::new();
    execution_results.insert(*deploy.id(), execution_result.clone());
    put_execution_results(&mut harness, &mut storage, *block.hash(), execution_results);

    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 42).expect("block not indexed properly"),
        *block
    );

    // After storing everything, destroy the harness and component, then rebuild using the same
    // directory as backing.
    let (on_disk, rng) = harness.into_parts();
    let mut harness = ComponentHarness::builder()
        .on_disk(on_disk)
        .rng(rng)
        .build();
    let mut storage = storage_fixture(&harness);

    let actual_block = get_block(&mut harness, &mut storage, *block.hash())
        .expect("missing block we stored earlier");
    assert_eq!(actual_block, *block);

    let actual_deploys = get_deploys(&mut harness, &mut storage, smallvec![*deploy.id()]);
    assert_eq!(actual_deploys, vec![Some(deploy.clone())]);

    let (_, deploy_metadata) = get_deploy_and_metadata(&mut harness, &mut storage, *deploy.id())
        .expect("missing deploy we stored earlier");

    let execution_results = deploy_metadata.execution_results;
    assert_eq!(execution_results.len(), 1);
    assert_eq!(execution_results[block.hash()], execution_result);

    assert_eq!(
        get_block_at_height(&mut harness, &mut storage, 42).expect("block index was not restored"),
        *block
    );
}

#[test]
fn should_hard_reset() {
    let blocks_count = 8_usize;
    let blocks_per_era = 3;
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create and store 8 blocks, 0-2 in era 0, 3-5 in era 1, and 6,7 in era 2.
    let blocks: Vec<Block> = (0..blocks_count)
        .map(|height| {
            let is_switch = height % blocks_per_era == blocks_per_era - 1;
            Block::random_with_specifics(
                &mut harness.rng,
                EraId::from(height as u64 / 3),
                height as u64,
                ProtocolVersion::V1_0_0,
                is_switch,
            )
        })
        .collect();

    for block in &blocks {
        assert!(put_block(
            &mut harness,
            &mut storage,
            Box::new(block.clone())
        ));
    }

    // Create and store signatures for these blocks.
    for block in &blocks {
        let block_signatures = random_signatures(&mut harness.rng, block);
        assert!(put_block_signatures(
            &mut harness,
            &mut storage,
            block_signatures
        ));
    }

    // Add execution results to deploys; deploy 0 will be executed in block 0, deploy 1 in block 1,
    // and so on.
    let mut deploys = vec![];
    let mut execution_results = vec![];
    for block_hash in blocks.iter().map(|block| block.hash()) {
        let deploy = Deploy::random(&mut harness.rng);
        let execution_result: ExecutionResult = harness.rng.gen();
        let mut exec_results = HashMap::new();
        exec_results.insert(*deploy.id(), execution_result);
        put_deploy(&mut harness, &mut storage, Box::new(deploy.clone()));
        put_execution_results(
            &mut harness,
            &mut storage,
            *block_hash,
            exec_results.clone(),
        );
        deploys.push(deploy);
        execution_results.push(exec_results);
    }

    // Check the highest block is #7.
    assert_eq!(
        Some(blocks[blocks_count - 1].clone()),
        get_highest_block(&mut harness, &mut storage)
    );

    // The closure doing the actual checks.
    let mut check = |reset_era: usize| {
        // Initialize a new storage with a hard reset to the given era, deleting blocks from that
        // era onwards.
        let mut storage = storage_fixture_with_hard_reset(&harness, EraId::from(reset_era as u64));

        // Check highest block is the last from the previous era, or `None` if resetting to era 0.
        let highest_block = get_highest_block(&mut harness, &mut storage);
        if reset_era > 0 {
            assert_eq!(
                blocks[blocks_per_era * reset_era - 1],
                highest_block.unwrap()
            );
        } else {
            assert!(highest_block.is_none());
        }

        // Check deleted blocks can't be retrieved.
        for (index, block) in blocks.iter().enumerate() {
            let result = get_block(&mut harness, &mut storage, *block.hash());
            let should_get_block = index < blocks_per_era * reset_era;
            assert_eq!(should_get_block, result.is_some());
        }

        // Check signatures of deleted blocks can't be retrieved.
        for (index, block) in blocks.iter().enumerate() {
            let result = get_block_signatures(&mut harness, &mut storage, *block.hash());
            let should_get_sigs = index < blocks_per_era * reset_era;
            assert_eq!(should_get_sigs, result.is_some());
        }

        // Check execution results in deleted blocks have been removed.
        for (index, deploy) in deploys.iter().enumerate() {
            let (_deploy, metadata) =
                get_deploy_and_metadata(&mut harness, &mut storage, *deploy.id()).unwrap();
            let should_have_exec_results = index < blocks_per_era * reset_era;
            assert_eq!(
                should_have_exec_results,
                !metadata.execution_results.is_empty()
            );
        }
    };

    // Test with a hard reset to era 2, deleting blocks (and associated data) 6 and 7.
    check(2);
    // Test with a hard reset to era 1, further deleting blocks (and associated data) 3, 4 and 5.
    check(1);
    // Test with a hard reset to era 0, deleting all blocks and associated data.
    check(0);
}

#[derive(Debug, Clone, PartialEq)]
struct DatabaseEntriesSnapshot {
    block_body_keys: HashSet<Digest>,
    deploy_hashes_keys: HashSet<Digest>,
    transfer_hashes_keys: HashSet<Digest>,
    proposer_keys: HashSet<Digest>,
}

impl DatabaseEntriesSnapshot {
    fn from_storage(storage: &Storage) -> DatabaseEntriesSnapshot {
        let txn = storage.env.begin_ro_txn().unwrap();

        let mut cursor = txn.open_ro_cursor(storage.block_body_v2_db).unwrap();
        let block_body_keys = cursor
            .iter()
            .map(|(raw_key, _)| Digest::try_from(raw_key).unwrap())
            .collect();
        drop(cursor); // borrow checker complains without this

        let mut cursor = txn.open_ro_cursor(storage.deploy_hashes_db).unwrap();
        let deploy_hashes_keys = cursor
            .iter()
            .map(|(raw_key, _)| Digest::try_from(raw_key).unwrap())
            .collect();
        drop(cursor); // borrow checker complains without this

        let mut cursor = txn.open_ro_cursor(storage.transfer_hashes_db).unwrap();
        let transfer_hashes_keys = cursor
            .iter()
            .map(|(raw_key, _)| Digest::try_from(raw_key).unwrap())
            .collect();
        drop(cursor); // borrow checker complains without this

        let mut cursor = txn.open_ro_cursor(storage.proposer_db).unwrap();
        let proposer_keys = cursor
            .iter()
            .map(|(raw_key, _)| Digest::try_from(raw_key).unwrap())
            .collect();
        drop(cursor); // borrow checker complains without this

        txn.commit().unwrap();

        DatabaseEntriesSnapshot {
            block_body_keys,
            deploy_hashes_keys,
            transfer_hashes_keys,
            proposer_keys,
        }
    }
}

#[test]
fn should_garbage_collect() {
    let blocks_count = 9_usize;
    let blocks_per_era = 3;
    let mut harness = ComponentHarness::default();
    let mut storage = storage_fixture(&harness);

    // Create and store 8 blocks, 0-2 in era 0, 3-5 in era 1, and 6,7 in era 2.
    let blocks: Vec<Block> = (0..blocks_count)
        .map(|height| {
            let is_switch = height % blocks_per_era == blocks_per_era - 1;
            Block::random_with_specifics(
                &mut harness.rng,
                EraId::from((height / blocks_per_era) as u64),
                height as u64,
                ProtocolVersion::from_parts(1, 4, 0), // so that they are stored in v2 scheme
                is_switch,
            )
        })
        .collect();

    let mut snapshots = vec![];

    for block in &blocks {
        assert!(put_block(
            &mut harness,
            &mut storage,
            Box::new(block.clone())
        ));
        // store the storage state after a switch block for later comparison
        if block.header().is_switch_block() {
            snapshots.push(DatabaseEntriesSnapshot::from_storage(&storage));
        }
    }

    let check = |reset_era: usize| {
        // Initialize a new storage with a hard reset to the given era, deleting blocks from that
        // era onwards.
        let storage = storage_fixture_with_hard_reset_and_protocol_version(
            &harness,
            EraId::from(reset_era as u64),
            ProtocolVersion::from_parts(1, 5, 0), /* this is needed because blocks with later
                                                   * versions aren't removed on hard resets */
        );

        // Hard reset should remove headers, but not block bodies
        let snapshot = DatabaseEntriesSnapshot::from_storage(&storage);
        assert_eq!(snapshot, snapshots[reset_era]);

        // Run garbage collection
        let txn = storage.env.begin_ro_txn().unwrap();

        let block_header_map =
            construct_block_body_to_block_header_reverse_lookup(&txn, &storage.block_header_db)
                .unwrap();
        txn.commit().unwrap();

        let mut txn = storage.env.begin_rw_txn().unwrap();
        garbage_collect_block_body_v2_db(
            &mut txn,
            &storage.block_body_v2_db,
            &storage.deploy_hashes_db,
            &storage.transfer_hashes_db,
            &storage.proposer_db,
            &block_header_map,
        )
        .unwrap();
        txn.commit().unwrap();

        // Garbage collection after removal of blocks from era 2 should revert the state of block
        // bodies to what it was after era 1.
        let snapshot = DatabaseEntriesSnapshot::from_storage(&storage);
        assert_eq!(snapshot, snapshots[reset_era - 1]);
    };

    check(2);
    check(1);
}
