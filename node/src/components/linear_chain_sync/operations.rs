use std::{
    collections::BTreeMap,
    fmt::Debug,
    ops::{Add, Div},
    time::Duration,
};

use num::rational::Ratio;
use tracing::{error, info, trace, warn};

use casper_execution_engine::{
    shared::{newtypes::Blake2bHash, stored_value::StoredValue},
    storage::trie::Trie,
};
use casper_types::{EraId, Key, PublicKey, U512};

use crate::{
    components::{
        consensus,
        fetcher::{FetchedData, FetcherError},
        linear_chain_sync::error::{FinalitySignatureError, LinearChainSyncError},
    },
    effect::{
        requests::{ContractRuntimeRequest, FetcherRequest, NetworkInfoRequest, StorageRequest},
        EffectBuilder,
    },
    types::{
        BlockHash, BlockHeader, BlockHeaderWithMetadata, BlockSignatures, BlockWithMetadata,
        Chainspec, Deploy, DeployHash, Item, Timestamp,
    },
};

const TIMEOUT_DURATION: Duration = Duration::from_millis(100);

/// Fetches an item. Keeps retrying to fetch until it is successful. Assumes no integrity check is
/// necessary for the item. Not suited to fetching a block header or block by height, which require
/// verification with finality signatures.
async fn fetch_retry_forever<T, REv, I>(
    effect_builder: EffectBuilder<REv>,
    id: T::Id,
) -> Result<Box<T>, FetcherError<T, I>>
where
    T: Item + 'static,
    REv: From<FetcherRequest<I, T>> + From<NetworkInfoRequest<I>>,
    I: Eq + Debug + Clone + Send + 'static,
{
    loop {
        for peer in effect_builder.get_peers_in_random_order().await {
            trace!(
                "Attempting to fetch {:?} with id {:?} from {:?}",
                T::TAG,
                id,
                peer
            );
            match effect_builder.fetch::<T, I>(id, peer.clone()).await {
                Ok(FetchedData::FromStorage { item }) => {
                    trace!(
                        "Did not get {:?} with id {:?} from {:?}, got from storage instead",
                        T::TAG,
                        id,
                        peer
                    );
                    return Ok(item);
                }
                Ok(FetchedData::FromPeer { item, .. }) => {
                    trace!("Fetched {:?} with id {:?} from {:?}", T::TAG, id, peer);
                    return Ok(item);
                }
                Err(FetcherError::AbsentFromPeer { .. }) => {
                    warn!(
                        ?id,
                        tag = ?T::TAG,
                        ?peer,
                        "Fast sync could not fetch; trying next peer",
                    )
                }
                Err(FetcherError::TimedOutFromPeer { .. }) => {
                    warn!(
                        ?id,
                        tag = ?T::TAG,
                        ?peer,
                        "Peer timed out",
                    );
                    // Peer timed out fetching the item, continue with the next peer
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        tokio::time::sleep(TIMEOUT_DURATION).await
    }
}

/// Fetches and stores a block header from the network
async fn fetch_and_store_block_header<REv, I>(
    effect_builder: EffectBuilder<REv>,
    block_hash: BlockHash,
) -> Result<Box<BlockHeader>, FetcherError<BlockHeader, I>>
where
    REv: From<FetcherRequest<I, BlockHeader>> + From<NetworkInfoRequest<I>> + From<StorageRequest>,
    I: Eq + Debug + Clone + Send + 'static,
{
    let block_header =
        fetch_retry_forever::<BlockHeader, REv, I>(effect_builder, block_hash).await?;
    effect_builder
        .put_block_header_to_storage(block_header.clone())
        .await;
    Ok(block_header)
}

/// Fetches and stores a deploy.
async fn fetch_and_store_deploy<REv, I>(
    effect_builder: EffectBuilder<REv>,
    deploy_or_transfer_hash: DeployHash,
) -> Result<Box<Deploy>, FetcherError<Deploy, I>>
where
    REv: From<FetcherRequest<I, Deploy>> + From<NetworkInfoRequest<I>> + From<StorageRequest>,
    I: Eq + Debug + Clone + Send + 'static,
{
    let deploy =
        fetch_retry_forever::<Deploy, REv, I>(effect_builder, deploy_or_transfer_hash).await?;
    effect_builder.put_deploy_to_storage(deploy.clone()).await;
    Ok(deploy)
}

/// Returns the genesis validator weights, by public key.
fn get_genesis_validators(chainspec: &Chainspec) -> BTreeMap<PublicKey, U512> {
    chainspec
        .network_config
        .chainspec_validator_stakes()
        .into_iter()
        .map(|(pub_key, motes)| (pub_key, motes.value()))
        .collect()
}

/// Get trusted switch block; returns `None` if we are still in the first era.
async fn maybe_get_trusted_switch_block<REv, I>(
    effect_builder: EffectBuilder<REv>,
    chainspec: &Chainspec,
    trusted_header: &BlockHeader,
) -> Result<Option<BlockHeader>, LinearChainSyncError<I>>
where
    REv: From<FetcherRequest<I, BlockHeader>> + From<NetworkInfoRequest<I>> + From<StorageRequest>,
    I: Eq + Debug + Clone + Send + 'static,
{
    // If we are still in the first era, there is no switch block.
    if trusted_header.era_id().is_genesis() {
        return Ok(None);
    }

    // Check that we are not restarting right after an emergency restart, which is too early
    // Consider emitting a switch block after an emergency restart to make this simpler...
    let maybe_last_emergency_restart_era_id = chainspec.protocol_config.last_emergency_restart;

    let min_era = maybe_last_emergency_restart_era_id.unwrap_or_else(|| EraId::new(0));
    if min_era >= trusted_header.era_id() {
        return Err(LinearChainSyncError::TrustedHeaderEraTooEarly {
            trusted_header: Box::new(trusted_header.clone()),
            maybe_last_emergency_restart_era_id,
        });
    };

    // Fetch each parent hash one by one until we have the trusted validator weights
    let mut current_header_to_walk_back_from = trusted_header.clone();
    while !current_header_to_walk_back_from.is_switch_block() {
        current_header_to_walk_back_from = *fetch_and_store_block_header(
            effect_builder,
            *current_header_to_walk_back_from.parent_hash(),
        )
        .await?;
    }
    Ok(Some(current_header_to_walk_back_from))
}

/// Verifies finality signatures for a block header
fn validate_finality_signatures(
    block_header: &BlockHeader,
    trusted_validator_weights: &BTreeMap<PublicKey, U512>,
    finality_threshold_fraction: Ratio<u64>,
    block_signatures: &BlockSignatures,
) -> Result<(), FinalitySignatureError> {
    // Check the signatures' block hash is the header's block hash
    let block_hash = block_header.hash();
    if block_signatures.block_hash != block_hash {
        return Err(
            FinalitySignatureError::SignaturesDoNotCorrespondToBlockHeader {
                block_header: Box::new(block_header.clone()),
                block_hash: Box::new(block_hash),
                block_signatures: Box::new(block_signatures.clone()),
            },
        );
    }

    // Cryptographically verify block signatures
    block_signatures.verify()?;
    weigh_finality_signatures(
        trusted_validator_weights,
        finality_threshold_fraction,
        block_signatures,
    )
}

pub(crate) fn weigh_finality_signatures(
    trusted_validator_weights: &BTreeMap<PublicKey, U512>,
    finality_threshold_fraction: Ratio<u64>,
    block_signatures: &BlockSignatures,
) -> Result<(), FinalitySignatureError> {
    // Calculate the weight of the signatures
    let mut signature_weight: U512 = U512::zero();
    for (public_key, _) in block_signatures.proofs.iter() {
        match trusted_validator_weights.get(public_key) {
            None => {
                return Err(FinalitySignatureError::BogusValidator {
                    trusted_validator_weights: trusted_validator_weights.clone(),
                    block_signatures: Box::new(block_signatures.clone()),
                    bogus_validator_public_key: Box::new(public_key.clone()),
                })
            }
            Some(validator_weight) => {
                signature_weight += *validator_weight;
            }
        }
    }

    // Check the finality signatures have sufficient weight
    let total_weight: U512 = trusted_validator_weights
        .iter()
        .map(|(_, weight)| *weight)
        .sum();

    let lower_bound = finality_threshold_fraction.add(1).div(2);
    // Verify: signature_weight / total_weight >= lower_bound
    // Equivalent to the following
    if signature_weight * U512::from(*lower_bound.denom())
        <= total_weight * U512::from(*lower_bound.numer())
    {
        return Err(FinalitySignatureError::InsufficientWeightForFinality {
            trusted_validator_weights: trusted_validator_weights.clone(),
            block_signatures: Box::new(block_signatures.clone()),
            signature_weight: Box::new(signature_weight),
            total_validator_weight: Box::new(total_weight),
            finality_threshold_fraction,
        });
    }

    Ok(())
}

/// Fetches a block header from the network by height.
async fn fetch_and_store_block_header_by_height<REv, I>(
    effect_builder: EffectBuilder<REv>,
    height: u64,
    trusted_validator_weights: &BTreeMap<PublicKey, U512>,
    finality_threshold_fraction: Ratio<u64>,
) -> Result<Option<Box<BlockHeaderWithMetadata>>, FetcherError<BlockHeaderWithMetadata, I>>
where
    REv: From<FetcherRequest<I, BlockHeaderWithMetadata>>
        + From<NetworkInfoRequest<I>>
        + From<StorageRequest>,
    I: Eq + Debug + Clone + Send + 'static,
{
    for peer in effect_builder.get_peers_in_random_order().await {
        match effect_builder
            .fetch::<BlockHeaderWithMetadata, I>(height, peer.clone())
            .await
        {
            Ok(FetchedData::FromStorage { item }) => return Ok(Some(item)),
            Ok(FetchedData::FromPeer { item, .. }) => {
                let BlockHeaderWithMetadata {
                    block_header,
                    block_signatures,
                } = *item.clone();

                if let Err(error) = validate_finality_signatures(
                    &block_header,
                    &trusted_validator_weights,
                    finality_threshold_fraction,
                    &block_signatures,
                ) {
                    warn!(
                        ?error,
                        ?peer,
                        "Error validating finality signatures from peer.",
                    );
                    // TODO: ban peer
                    continue;
                }

                // Store the block header
                effect_builder
                    .put_block_header_to_storage(Box::new(block_header.clone()))
                    .await;

                // Store the finality signatures
                effect_builder
                    .put_signatures_to_storage(block_signatures.clone())
                    .await;

                return Ok(Some(item));
            }
            Err(FetcherError::AbsentFromPeer { .. }) => {
                warn!(
                    height,
                    tag = ?BlockHeaderWithMetadata::TAG,
                    ?peer,
                    "Fast sync could not fetch",
                );
                // If the peer we requested doesn't have the item, continue with the next peer
                continue;
            }
            Err(FetcherError::TimedOutFromPeer { .. }) => {
                warn!(
                    height,
                    tag = ?BlockHeaderWithMetadata::TAG,
                    ?peer,
                    "Peer timed out",
                );
                // Peer timed out fetching the item, continue with the next peer
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// Fetches a block from the network by height.
async fn fetch_and_store_block_by_height<REv, I>(
    effect_builder: EffectBuilder<REv>,
    height: u64,
    trusted_validator_weights: &BTreeMap<PublicKey, U512>,
    finality_threshold_fraction: Ratio<u64>,
) -> Result<Option<Box<BlockWithMetadata>>, FetcherError<BlockWithMetadata, I>>
where
    REv: From<FetcherRequest<I, BlockWithMetadata>>
        + From<NetworkInfoRequest<I>>
        + From<StorageRequest>,
    I: Eq + Debug + Clone + Send + 'static,
{
    for peer in effect_builder.get_peers_in_random_order().await {
        match effect_builder
            .fetch::<BlockWithMetadata, I>(height, peer.clone())
            .await
        {
            Ok(FetchedData::FromStorage { item }) => return Ok(Some(item)),
            Ok(FetchedData::FromPeer { item, .. }) => {
                let BlockWithMetadata {
                    block,
                    finality_signatures,
                } = &*item;

                if let Err(error) = block.verify() {
                    warn!(
                        ?error,
                        ?peer,
                        "Error validating finality signatures from peer.",
                    );
                    // TODO: ban peer
                    continue;
                }

                if let Err(error) = validate_finality_signatures(
                    block.header(),
                    &trusted_validator_weights,
                    finality_threshold_fraction,
                    &finality_signatures,
                ) {
                    warn!(
                        ?error,
                        ?peer,
                        "Error validating finality signatures from peer.",
                    );
                    // TODO: ban peer
                    continue;
                }

                // Store the block
                // effect_builder
                //     .put_block_to_storage(Box::new(block.clone()))
                //     .await;

                // Store the finality signatures
                effect_builder
                    .put_signatures_to_storage(finality_signatures.clone())
                    .await;

                return Ok(Some(item));
            }
            Err(FetcherError::AbsentFromPeer { .. }) => {
                warn!(
                    height,
                    tag = ?BlockWithMetadata::TAG,
                    ?peer,
                    "Block by height absent from peer",
                );
                // If the peer we requested doesn't have the item, continue with the next peer
                continue;
            }
            Err(FetcherError::TimedOutFromPeer { .. }) => {
                warn!(
                    height,
                    tag = ?BlockWithMetadata::TAG,
                    ?peer,
                    "Peer timed out",
                );
                // Peer timed out fetching the item, continue with the next peer
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// Queries all of the peers for a trie, puts the trie found from the network in the trie-store, and
/// returns any outstanding descendant tries.
async fn fetch_trie_and_insert_into_trie_store<REv, I>(
    effect_builder: EffectBuilder<REv>,
    trie_key: Blake2bHash,
) -> Result<Vec<Blake2bHash>, LinearChainSyncError<I>>
where
    REv: From<ContractRuntimeRequest>
        + From<FetcherRequest<I, Trie<Key, StoredValue>>>
        + From<NetworkInfoRequest<I>>,
    I: Eq + Clone + Debug + Send + 'static,
{
    let trie =
        fetch_retry_forever::<Trie<Key, StoredValue>, REv, I>(effect_builder, trie_key).await?;
    let outstanding_tries = effect_builder
        .put_trie_and_find_missing_descendant_trie_keys(trie)
        .await?;
    Ok(outstanding_tries)
}

/// Runs the fast synchronization task.
pub(crate) async fn run_fast_sync_task<REv, I>(
    effect_builder: EffectBuilder<REv>,
    trusted_hash: BlockHash,
    chainspec: Chainspec,
) -> Result<BlockHeader, LinearChainSyncError<I>>
where
    REv: From<ContractRuntimeRequest>
        + From<FetcherRequest<I, BlockHeader>>
        + From<FetcherRequest<I, BlockHeaderWithMetadata>>
        + From<FetcherRequest<I, BlockWithMetadata>>
        + From<FetcherRequest<I, Deploy>>
        + From<FetcherRequest<I, Trie<Key, StoredValue>>>
        + From<NetworkInfoRequest<I>>
        + From<StorageRequest>,
    I: Eq + Debug + Clone + Send + 'static,
{
    // Fetch the trusted header
    let trusted_header = fetch_and_store_block_header(effect_builder, trusted_hash).await?;

    // TODO: This will get the pre-upgrade switch block even after an emergency restart. Use the
    // post-upgrade validator set instead.
    let mut maybe_trusted_switch_block =
        maybe_get_trusted_switch_block(effect_builder, &chainspec, &trusted_header).await?;

    let mut trusted_validator_weights = maybe_trusted_switch_block.as_ref().map_or_else(
        || get_genesis_validators(&chainspec),
        |switch_block| switch_block.next_era_validator_weights().unwrap().clone(),
    );

    // Get the most recent header which has the same version as ours
    // We keep fetching by height until none of our peers have a block at that height, or we reach
    // a current era.
    let trusted_header_output = *trusted_header.clone();
    let mut most_recent_block_header = *trusted_header;
    let current_version = chainspec.protocol_config.version;
    loop {
        // If we encounter a block header of a version which is newer than ours we return an error
        if most_recent_block_header.protocol_version() > current_version {
            return Err(
                LinearChainSyncError::RetrievedBlockHeaderFromFutureVersion {
                    current_version,
                    block_header_with_future_version: Box::new(most_recent_block_header),
                },
            );
        }
        let maybe_fetched_block = fetch_and_store_block_header_by_height(
            effect_builder,
            most_recent_block_header.height() + 1,
            &trusted_validator_weights,
            chainspec.highway_config.finality_threshold_fraction,
        )
        .await?;
        match maybe_fetched_block {
            Some(more_recent_block_header_with_metadata) => {
                most_recent_block_header = more_recent_block_header_with_metadata.block_header;
                // If the new block is a switch block, update the validator weights
                if let Some(new_trusted_validator_weights) =
                    most_recent_block_header.next_era_validator_weights()
                {
                    trusted_validator_weights = new_trusted_validator_weights.clone();
                    maybe_trusted_switch_block = Some(most_recent_block_header.clone());
                }

                if is_current_era(
                    &most_recent_block_header,
                    maybe_trusted_switch_block.as_ref(),
                    &chainspec,
                    Timestamp::now(),
                )? {
                    info!(
                        era = most_recent_block_header.era_id().value(),
                        timestamp = %most_recent_block_header.timestamp(),
                        height = most_recent_block_header.height(),
                        "Reached a block in the current era",
                    );
                    break;
                }
            }
            // If we could not fetch, we can stop if the most recent has our protocol version
            None if most_recent_block_header.protocol_version() == current_version => break,
            // Otherwise keep trying to fetch until we get a block with our version
            None => {
                tokio::time::sleep(TIMEOUT_DURATION).await;
            }
        }
    }

    // Synchronize the trie store for the most recent block header.

    // Corner case: if an emergency restart happened recently, it is necessary to synchronize the
    // state for the block right after the emergency restart.  This is needed for the EraSupervisor.

    // The era supervisor needs validator information from previous eras it may potentially slash.
    // The number of previous eras is determined by a *delay* in which consensus participants become
    // bonded validators or unbond.
    let delay = consensus::bonded_eras(&(&chainspec).into());
    // The era supervisor requires at least to 3*delay + 1 eras back to be stored in the database.
    let historical_eras_needed = delay.saturating_mul(3).saturating_add(1);

    let maybe_last_emergency_restart_era_id = chainspec.protocol_config.last_emergency_restart;
    match maybe_last_emergency_restart_era_id {
        Some(last_emergency_restart_era_id)
            if last_emergency_restart_era_id
                > most_recent_block_header
                    .era_id()
                    .saturating_sub(historical_eras_needed) =>
        {
            // Walk backwards until we have the first block after the emergency upgrade
            loop {
                let previous_block_header = fetch_and_store_block_header(
                    effect_builder,
                    *most_recent_block_header.parent_hash(),
                )
                .await?;
                if previous_block_header.era_id() == last_emergency_restart_era_id.saturating_sub(1)
                    && previous_block_header.is_switch_block()
                {
                    break;
                }
                most_recent_block_header = *previous_block_header;
            }
        }
        _ => {}
    }

    // Use the state root to synchronize the trie.
    info!(
        state_root_hash = ?most_recent_block_header.state_root_hash(),
        "Syncing trie store",
    );
    let mut outstanding_trie_keys = vec![Blake2bHash::from(
        *most_recent_block_header.state_root_hash(),
    )];
    while let Some(trie_key) = outstanding_trie_keys.pop() {
        let missing_descendant_trie_keys =
            fetch_trie_and_insert_into_trie_store(effect_builder, trie_key).await?;
        outstanding_trie_keys.extend(missing_descendant_trie_keys);
    }

    info!(
        most_recent_block_hash = ?most_recent_block_header.hash(),
        "Syncing blocks to current",
    );
    while let Some(block_with_metadata) = fetch_and_store_block_by_height(
        effect_builder,
        most_recent_block_header.height() + 1,
        &trusted_validator_weights,
        chainspec.highway_config.finality_threshold_fraction,
    )
    .await?
    {
        if block_with_metadata.block.protocol_version() > current_version {
            return Err(
                LinearChainSyncError::RetrievedBlockHeaderFromFutureVersion {
                    current_version,
                    block_header_with_future_version: Box::new(
                        block_with_metadata.block.take_header(),
                    ),
                },
            );
        }

        for deploy_hash in block_with_metadata.block.deploy_hashes() {
            fetch_and_store_deploy(effect_builder, *deploy_hash).await?;
        }

        for transfer_hash in block_with_metadata.block.transfer_hashes() {
            fetch_and_store_deploy(effect_builder, *transfer_hash).await?;
        }

        most_recent_block_header = block_with_metadata.block.take_header();
        if let Some(new_trusted_validator_weights) =
            most_recent_block_header.next_era_validator_weights()
        {
            trusted_validator_weights = new_trusted_validator_weights.clone();
        }
    }

    Ok(trusted_header_output)
}

/// Returns `true` if `most_recent_block` belongs to an era that is still ongoing.
pub(crate) fn is_current_era<I>(
    most_recent_block: &BlockHeader,
    maybe_switch_block: Option<&BlockHeader>,
    chainspec: &Chainspec,
    now: Timestamp,
) -> Result<bool, LinearChainSyncError<I>>
where
    I: Eq + Debug,
{
    // Compute the start timestamp of the current era, and the number of blocks so far.
    let (blocks_in_this_era, era_start) = match maybe_switch_block {
        Some(switch_block) => (
            most_recent_block.height() - switch_block.height(),
            switch_block.timestamp(),
        ),
        None => (
            most_recent_block.height() + 1,
            chainspec
                .protocol_config
                .activation_point
                .genesis_timestamp()
                .ok_or_else(|| {
                    error!(
                        ?most_recent_block,
                        "no recent switch block and no genesis timestamp"
                    );
                    LinearChainSyncError::MissingGenesisTimestamp
                })?,
        ),
    };

    // If the minimum era duration has not yet run out, the era is still current.
    if now.saturating_diff(era_start) < chainspec.core_config.era_duration {
        return Ok(true);
    }

    // Otherwise estimate the earliest possible end of this era based on how many blocks remain.
    let remaining_blocks_in_this_era = chainspec
        .core_config
        .minimum_era_height
        .saturating_sub(blocks_in_this_era);
    let min_round_length = chainspec.highway_config.min_round_length();
    let time_since_most_recent_block = now.saturating_diff(most_recent_block.timestamp());
    Ok(time_since_most_recent_block < min_round_length * remaining_blocks_in_this_era)
}

#[cfg(test)]
mod tests {
    use std::iter;

    use super::*;

    use casper_types::{PublicKey, SecretKey};

    use crate::{
        components::consensus::EraReport,
        crypto::AsymmetricKeyExt,
        types::{Block, BlockPayload, FinalizedBlock},
        utils::Loadable,
    };

    /// Creates a block for testing, with the given data, and returns its header.
    ///
    /// The other fields are filled in with defaults, since they are not used in these tests.
    fn create_block(
        timestamp: Timestamp,
        era_id: EraId,
        height: u64,
        switch_block: bool,
    ) -> BlockHeader {
        let secret_key = SecretKey::doc_example();
        let public_key = PublicKey::from(secret_key);

        let maybe_era_report = switch_block.then(|| EraReport {
            equivocators: Default::default(),
            rewards: Default::default(),
            inactive_validators: Default::default(),
        });
        let next_era_validator_weights =
            switch_block.then(|| iter::once((public_key.clone(), 100.into())).collect());

        let block_payload = BlockPayload::new(
            Default::default(), // deploy hashes
            Default::default(), // transfer hashes
            Default::default(), // accusations
            false,              // random bit
        );

        let finalized_block = FinalizedBlock::new(
            block_payload,
            maybe_era_report,
            timestamp,
            era_id,
            height,
            public_key,
        );
        Block::new(
            Default::default(), // parent block hash
            Default::default(), // parent random seed
            Default::default(), // state root hash
            finalized_block,
            next_era_validator_weights,
            Default::default(), // protocol version
        )
        .expect("failed to create block for tests")
        .take_header()
    }

    #[test]
    fn test_is_current_era() {
        let mut chainspec = Chainspec::from_resources("local");

        let genesis_time = chainspec
            .protocol_config
            .activation_point
            .genesis_timestamp()
            .expect("test expects genesis timestamp in chainspec");
        let min_round_length = chainspec.highway_config.min_round_length();

        // Configure eras to have at least 10 blocks but to last at least 20 minimum-length rounds.
        let era_duration = min_round_length * 20;
        let minimum_era_height = 10;
        chainspec.core_config.era_duration = era_duration;
        chainspec.core_config.minimum_era_height = minimum_era_height;

        // We assume era 6 started after six minimum era durations, at block 100.
        let era6_start = genesis_time + era_duration * 6;
        let switch_block5 = create_block(era6_start, EraId::from(5), 100, true);

        // If we are still within the minimum era duration the era is current, even if we have the
        // required number of blocks (115 - 100 > 10).
        let block_time = era6_start + era_duration - 10.into();
        let now = block_time + 5.into();
        let block = create_block(block_time, EraId::from(6), 115, false);
        assert!(is_current_era::<()>(&block, Some(&switch_block5), &chainspec, now).unwrap());

        // If the minimum duration has passed but we we know we don't have all blocks yet, it's
        // also still current. There are still five blocks missing but only four rounds have
        // passed.
        let block_time = era6_start + era_duration * 2;
        let now = block_time + min_round_length * 4;
        let block = create_block(block_time, EraId::from(6), 105, false);
        assert!(is_current_era::<()>(&block, Some(&switch_block5), &chainspec, now).unwrap());

        // If both criteria are satisfied, the era could have ended.
        let block_time = era6_start + era_duration * 2;
        let now = block_time + min_round_length * 5;
        let block = create_block(block_time, EraId::from(6), 105, false);
        assert!(!is_current_era::<()>(&block, Some(&switch_block5), &chainspec, now).unwrap());

        // Now we do the same tests in era 0. In that case, the switch block is None.

        // If we are still within the minimum era duration the era is current, even if we have the
        // required number of blocks (14 > 10).
        let block_time = genesis_time + era_duration - 10.into();
        let now = block_time + 5.into();
        let block = create_block(block_time, EraId::from(0), 14, false);
        assert!(is_current_era::<()>(&block, None, &chainspec, now).unwrap());

        // If the minimum duration has passed but we we know we don't have all blocks yet, it's
        // also still current. There are still five blocks missing but only three rounds have
        // passed. (Block 4 is the fifth block.)
        let block_time = genesis_time + era_duration * 2;
        let now = block_time + min_round_length * 4;
        let block = create_block(block_time, EraId::from(0), 4, false);
        assert!(is_current_era::<()>(&block, None, &chainspec, now).unwrap());

        // If both criteria are satisfied, the era could have ended.
        let block_time = genesis_time + era_duration * 2;
        let now = block_time + min_round_length * 5;
        let block = create_block(block_time, EraId::from(0), 4, false);
        assert!(!is_current_era::<()>(&block, None, &chainspec, now).unwrap());
    }
}
