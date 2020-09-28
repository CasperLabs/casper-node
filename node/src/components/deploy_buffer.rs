//! Deploy buffer.
//!
//! The deploy buffer stores deploy hashes in memory, tracking their suitability for inclusion into
//! a new block. Upon request, it returns a list of candidates that can be included.

use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Display, Formatter},
    sync::{Arc, Mutex},
    time::Duration,
};

use datasize::DataSize;
use derive_more::From;
use rand::{CryptoRng, Rng};
use semver::Version;
use tracing::{error, info};

use crate::{
    components::{chainspec_loader::DeployConfig, storage::Storage, Component},
    effect::{
        requests::{DeployBufferRequest, StorageRequest},
        EffectBuilder, EffectExt, Effects, Responder,
    },
    types::{DeployHash, DeployHeader, ProtoBlock, ProtoBlockHash, Timestamp},
    Chainspec,
};

const DEPLOY_BUFFER_PRUNE_INTERVAL: Duration = Duration::from_secs(10);

/// An event for when using the deploy buffer as a component.
#[derive(Debug, From)]
pub enum Event {
    #[from]
    Request(DeployBufferRequest),
    /// A new deploy should be buffered.
    Buffer {
        hash: DeployHash,
        header: Box<DeployHeader>,
    },
    /// A proto block has been proposed. We should not propose duplicates of its deploys.
    ProposedProtoBlock(ProtoBlock),
    /// A proto block has been finalized. We should never propose its deploys again.
    FinalizedProtoBlock(ProtoBlock),
    /// A proto block has been orphaned. Its deploys should be re-proposed.
    OrphanedProtoBlock(ProtoBlock),
    /// The result of the `DeployBuffer` getting the chainspec from the storage component.
    GetChainspecResult {
        maybe_chainspec: Box<Option<Chainspec>>,
        current_instant: Timestamp,
        past_blocks: HashSet<ProtoBlockHash>,
        responder: Responder<HashSet<DeployHash>>,
    },
}

impl Display for Event {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::Request(req) => write!(f, "deploy-buffer request: {}", req),
            Event::Buffer { hash, .. } => write!(f, "deploy-buffer add {}", hash),
            Event::ProposedProtoBlock(block) => {
                write!(f, "deploy-buffer proposed proto block {}", block)
            }
            Event::FinalizedProtoBlock(block) => {
                write!(f, "deploy-buffer finalized proto block {}", block)
            }
            Event::OrphanedProtoBlock(block) => {
                write!(f, "deploy-buffer orphaned proto block {}", block)
            }
            Event::GetChainspecResult {
                maybe_chainspec, ..
            } => {
                if maybe_chainspec.is_some() {
                    write!(f, "deploy-buffer got chainspec")
                } else {
                    write!(f, "deploy-buffer failed to get chainspec")
                }
            }
        }
    }
}

type DeployCollection = HashMap<DeployHash, DeployHeader>;
type ProtoBlockCollection = HashMap<ProtoBlockHash, DeployCollection>;
/// Deploy buffer.
#[derive(DataSize, Debug, Default, Clone)]
struct DeployBufferInner {
    block_max_deploy_count: usize,
    collected_deploys: DeployCollection,
    processed: ProtoBlockCollection,
    finalized: ProtoBlockCollection,
}

/// Deploy buffer.
#[derive(DataSize, Debug, Clone)]
pub(crate) struct DeployBuffer {
    inner: Arc<Mutex<DeployBufferInner>>, // Don't hand out references to internal mutable state
}

impl DeployBuffer {
    /// Creates a new, empty deploy buffer instance.
    pub(crate) fn new(block_max_deploy_count: usize) -> Self {
        let inner = Arc::new(Mutex::new(DeployBufferInner {
            block_max_deploy_count,
            ..Default::default()
        }));
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Some(buffer) = weak.upgrade() {
                let pruned_items = buffer.lock().unwrap().prune();
                log::info!("{} items pruned from deploy buffer", pruned_items);
                tokio::time::delay_for(DEPLOY_BUFFER_PRUNE_INTERVAL).await;
            }
            log::debug!("DeployBuffer dropped");
        });
        Self { inner }
    }
}

#[cfg(test)]
impl DeployBuffer {
    fn remaining_deploys(
        &mut self,
        deploy_config: DeployConfig,
        current_instant: Timestamp,
        past_blocks: HashSet<ProtoBlockHash>,
    ) -> HashSet<DeployHash> {
        self.inner
            .lock()
            .unwrap()
            .remaining_deploys(deploy_config, current_instant, past_blocks)
    }

    fn add_deploy(&mut self, hash: DeployHash, header: DeployHeader) {
        self.inner.lock().unwrap().add_deploy(hash, header)
    }

    fn added_block<I>(&mut self, block: ProtoBlockHash, deploys: I)
    where
        I: IntoIterator<Item = DeployHash>,
    {
        self.inner.lock().unwrap().added_block(block, deploys)
    }
    /// Notifies the deploy buffer that a block has been finalized.
    fn finalized_block(&mut self, block: ProtoBlockHash) {
        self.inner.lock().unwrap().finalized_block(block)
    }
}

impl DeployBufferInner {
    /// Adds a deploy to the deploy buffer.
    ///
    /// Returns `false` if the deploy has been rejected.
    fn add_deploy(&mut self, hash: DeployHash, header: DeployHeader) {
        // only add the deploy if it isn't contained in a finalized block
        if !self
            .finalized
            .values()
            .any(|block| block.contains_key(&hash))
        {
            self.collected_deploys.insert(hash, header);
            info!("added deploy {} to the buffer", hash);
        } else {
            info!("deploy {} rejected from the buffer", hash);
        }
    }

    /// Gets the chainspec from storage in order to call `remaining_deploys()`.
    fn get_chainspec_from_storage<REv>(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        current_instant: Timestamp,
        past_blocks: HashSet<ProtoBlockHash>,
        responder: Responder<HashSet<DeployHash>>,
    ) -> Effects<Event>
    where
        REv: From<StorageRequest<Storage>> + Send,
    {
        // TODO - should the current protocol version be passed in here?
        let version = Version::from((1, 0, 0));
        effect_builder
            .get_chainspec(version)
            .event(move |maybe_chainspec| Event::GetChainspecResult {
                maybe_chainspec: Box::new(maybe_chainspec),
                current_instant,
                past_blocks,
                responder,
            })
    }

    /// Returns a list of candidates for inclusion into a block.
    fn remaining_deploys(
        &mut self,
        deploy_config: DeployConfig,
        current_instant: Timestamp,
        past_blocks: HashSet<ProtoBlockHash>,
    ) -> HashSet<DeployHash> {
        let past_deploys = past_blocks
            .iter()
            .filter_map(|block_hash| self.processed.get(block_hash))
            .chain(self.finalized.values())
            .flat_map(|deploys| deploys.keys())
            .collect::<HashSet<_>>();
        // deploys_to_return = all deploys in collected_deploys that aren't in finalized blocks or
        // processed blocks from the set `past_blocks`
        self.collected_deploys
            .iter()
            .filter(|&(hash, deploy)| {
                self.is_deploy_valid(deploy, current_instant, &deploy_config, &past_deploys)
                    && !past_deploys.contains(hash)
            })
            .map(|(hash, _deploy)| *hash)
            .take(self.block_max_deploy_count)
            .collect::<HashSet<_>>()
        // TODO: check gas and block size limits
    }

    /// Checks if a deploy is valid (for inclusion into the next block).
    fn is_deploy_valid(
        &self,
        deploy: &DeployHeader,
        current_instant: Timestamp,
        deploy_config: &DeployConfig,
        past_deploys: &HashSet<&DeployHash>,
    ) -> bool {
        let all_deps_resolved = || {
            deploy
                .dependencies()
                .iter()
                .all(|dep| past_deploys.contains(dep))
        };
        let ttl_valid = deploy.ttl() <= deploy_config.max_ttl;
        let timestamp_valid = deploy.timestamp() <= current_instant;
        let deploy_valid = deploy.timestamp() + deploy.ttl() >= current_instant;
        let num_deps_valid = deploy.dependencies().len() <= deploy_config.max_dependencies as usize;
        ttl_valid && timestamp_valid && deploy_valid && num_deps_valid && all_deps_resolved()
    }

    /// Notifies the deploy buffer of a new block that has been proposed, so that the block's
    /// deploys are not returned again by `remaining_deploys`.
    fn added_block<I>(&mut self, block: ProtoBlockHash, deploys: I)
    where
        I: IntoIterator<Item = DeployHash>,
    {
        // TODO: This will ignore deploys that weren't in `collected_deploys`. They might be added
        // later, and then would be proposed as duplicates.
        let deploy_map: HashMap<_, _> = deploys
            .into_iter()
            .filter_map(|deploy_hash| {
                self.collected_deploys
                    .get(&deploy_hash)
                    .map(|deploy| (deploy_hash, deploy.clone()))
            })
            .collect();
        self.collected_deploys
            .retain(|deploy_hash, _| !deploy_map.contains_key(deploy_hash));
        self.processed.insert(block, deploy_map);
    }

    /// Notifies the deploy buffer that a block has been finalized.
    fn finalized_block(&mut self, block: ProtoBlockHash) {
        if let Some(deploys) = self.processed.remove(&block) {
            self.collected_deploys
                .retain(|deploy_hash, _| !deploys.contains_key(deploy_hash));
            self.finalized.insert(block, deploys);
        } else if !block.is_empty() {
            // TODO: Events are not guaranteed to be handled in order, so this could happen!
            error!("finalized block that hasn't been processed!");
        }
    }

    /// Notifies the deploy buffer that a block has been orphaned.
    fn orphaned_block(&mut self, block: ProtoBlockHash) {
        if let Some(deploys) = self.processed.remove(&block) {
            self.collected_deploys.extend(deploys);
        } else {
            // TODO: Events are not guaranteed to be handled in order, so this could happen!
            error!("orphaned block that hasn't been processed!");
        }
    }

    /// Prunes stale deploy information from the DeployBuffer
    fn prune(&mut self) -> usize {
        /// Prunes DeployCollection and return the total (DeployHash, DeployHeader) entries pruned
        fn prune_collection(map: &mut DeployCollection) -> usize {
            let initial_len = map.len();
            map.retain(|_hash, header| {
                let now = Timestamp::now();
                let lifespan = header.timestamp() + header.ttl();
                lifespan > now
            });
            initial_len - map.len()
        }
        /// Prunes ProtoBlockCollection and return the total (DeployHash, DeployHeader) entries
        /// pruned
        fn prune_proto_collection(proto_collection: &mut ProtoBlockCollection) -> usize {
            let mut pruned = 0;
            let mut remove = Vec::new();
            for (proto_hash, processed) in proto_collection.iter_mut() {
                pruned += prune_collection(processed);
                if processed.is_empty() {
                    remove.push(*proto_hash);
                }
            }
            proto_collection.retain(|k, _v| !remove.contains(&k));
            pruned
        }
        let collected = prune_collection(&mut self.collected_deploys);
        let processed = prune_proto_collection(&mut self.processed);
        let finalized = prune_proto_collection(&mut self.finalized);
        collected + processed + finalized
    }
}

impl<REv, R> Component<REv, R> for DeployBuffer
where
    REv: From<StorageRequest<Storage>> + Send,
    R: Rng + CryptoRng + ?Sized,
{
    type Event = Event;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        rng: &mut R,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        self
            .inner
            .lock()
            .unwrap()
            .handle_event(effect_builder, rng, event)
    }
}

impl<REv, R> Component<REv, R> for DeployBufferInner
where
    REv: From<StorageRequest<Storage>> + Send,
    R: Rng + CryptoRng + ?Sized,
{
    type Event = Event;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut R,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::Request(DeployBufferRequest::ListForInclusion {
                current_instant,
                past_blocks,
                responder,
            }) => {
                return self.get_chainspec_from_storage(
                    effect_builder,
                    current_instant,
                    past_blocks,
                    responder,
                );
            }
            Event::Buffer { hash, header } => self.add_deploy(hash, *header),
            Event::ProposedProtoBlock(block) => {
                let (hash, deploys, _) = block.destructure();
                self.added_block(hash, deploys)
            }
            Event::FinalizedProtoBlock(block) => self.finalized_block(*block.hash()),
            Event::OrphanedProtoBlock(block) => self.orphaned_block(*block.hash()),
            Event::GetChainspecResult {
                maybe_chainspec,
                current_instant,
                past_blocks,
                responder,
            } => {
                let chainspec = maybe_chainspec.expect("should return chainspec");
                let deploys = self.remaining_deploys(
                    chainspec.genesis.deploy_config,
                    current_instant,
                    past_blocks,
                );
                return responder.respond(deploys).ignore();
            }
        }
        Effects::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use casper_execution_engine::core::engine_state::executable_deploy_item::ExecutableDeployItem;
    use rand::random;

    use super::*;
    use crate::{
        crypto::{asymmetric_key::SecretKey, hash::hash},
        testing::TestRng,
        types::{Deploy, DeployHash, DeployHeader, NodeConfig, ProtoBlockHash, TimeDiff},
    };

    fn generate_deploy(
        rng: &mut TestRng,
        timestamp: Timestamp,
        ttl: TimeDiff,
        dependencies: Vec<DeployHash>,
    ) -> (DeployHash, DeployHeader) {
        let secret_key = SecretKey::random(rng);
        let gas_price = 10;
        let chain_name = "chain".to_string();
        let payment = ExecutableDeployItem::ModuleBytes {
            module_bytes: vec![],
            args: vec![],
        };
        let session = ExecutableDeployItem::ModuleBytes {
            module_bytes: vec![],
            args: vec![],
        };

        let deploy = Deploy::new(
            timestamp,
            ttl,
            gas_price,
            dependencies,
            chain_name,
            payment,
            session,
            &secret_key,
            rng,
        );

        (*deploy.id(), deploy.take_header())
    }

    #[tokio::test] // Tests of DeployBuffer *must* be tokio::test as it relies on a tokio::time::delay
    async fn add_and_take_deploys() {
        let creation_time = Timestamp::from(100);
        let ttl = TimeDiff::from(100);
        let block_time1 = Timestamp::from(80);
        let block_time2 = Timestamp::from(120);
        let block_time3 = Timestamp::from(220);

        let no_blocks = HashSet::new();
        let mut buffer = DeployBuffer::new(NodeConfig::default().block_max_deploy_count as usize);
        let mut rng = TestRng::new();
        let (hash1, deploy1) = generate_deploy(&mut rng, creation_time, ttl, vec![]);
        let (hash2, deploy2) = generate_deploy(&mut rng, creation_time, ttl, vec![]);
        let (hash3, deploy3) = generate_deploy(&mut rng, creation_time, ttl, vec![]);
        let (hash4, deploy4) = generate_deploy(&mut rng, creation_time, ttl, vec![]);

        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time2, no_blocks.clone())
            .is_empty());

        // add two deploys
        buffer.add_deploy(hash1, deploy1);
        buffer.add_deploy(hash2, deploy2.clone());

        // if we try to create a block with a timestamp that is too early, we shouldn't get any
        // deploys
        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time1, no_blocks.clone())
            .is_empty());

        // if we try to create a block with a timestamp that is too late, we shouldn't get any
        // deploys, either
        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time3, no_blocks.clone())
            .is_empty());

        // take the deploys out
        let deploys =
            buffer.remaining_deploys(DeployConfig::default(), block_time2, no_blocks.clone());

        assert_eq!(deploys.len(), 2);
        assert!(deploys.contains(&hash1));
        assert!(deploys.contains(&hash2));

        // the deploys should not have been removed yet
        assert!(!buffer
            .remaining_deploys(DeployConfig::default(), block_time2, no_blocks.clone())
            .is_empty());

        // the two deploys will be included in block 1
        let block_hash1 = ProtoBlockHash::new(hash(random::<[u8; 16]>()));
        buffer.added_block(block_hash1, deploys);

        // the deploys should have been removed now
        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time2, no_blocks.clone())
            .is_empty());

        let mut blocks = HashSet::new();
        blocks.insert(block_hash1);

        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time2, blocks.clone())
            .is_empty());

        // try adding the same deploy again
        buffer.add_deploy(hash2, deploy2.clone());

        // it shouldn't be returned if we include block 1 in the past blocks
        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time2, blocks)
            .is_empty());
        // ...but it should be returned if we don't include it
        assert!(
            buffer
                .remaining_deploys(DeployConfig::default(), block_time2, no_blocks.clone())
                .len()
                == 1
        );

        // the previous check removed the deploy from the buffer, let's re-add it
        buffer.add_deploy(hash2, deploy2);

        // finalize the block
        buffer.finalized_block(block_hash1);

        // add more deploys
        buffer.add_deploy(hash3, deploy3);
        buffer.add_deploy(hash4, deploy4);

        let deploys = buffer.remaining_deploys(DeployConfig::default(), block_time2, no_blocks);

        // since block 1 is now finalized, deploy2 shouldn't be among the ones returned
        assert_eq!(deploys.len(), 2);
        assert!(deploys.contains(&hash3));
        assert!(deploys.contains(&hash4));
    }

    #[tokio::test]
    async fn test_prune() {
        let creation_time = Timestamp::from(100);
        let ttl = TimeDiff::from(100);

        let mut rng = TestRng::new();
        let (hash1, deploy1) = generate_deploy(&mut rng, creation_time, ttl, vec![]);
        let (hash2, deploy2) = generate_deploy(&mut rng, creation_time, ttl, vec![]);
        let mut buffer = DeployBuffer::new(NodeConfig::default().block_max_deploy_count as usize);

        // collected_deploys
        buffer.add_deploy(hash1.clone(), deploy1);

        let block_hash1 = ProtoBlockHash::new(hash(random::<[u8; 16]>()));
        // collected_deploys => processed
        buffer.added_block(block_hash1.clone(), vec![hash1]);

        // processed => finalized
        buffer.finalized_block(block_hash1.clone());

        buffer.add_deploy(hash2.clone(), deploy2);
        let block_hash2 = ProtoBlockHash::new(hash(random::<[u8; 16]>()));
        buffer.added_block(block_hash2.clone(), vec![hash2]);

        {
            let inner = buffer.inner.lock().unwrap();
            assert_eq!(inner.collected_deploys.len(), 0);
            assert_eq!(inner.processed.get(&block_hash2).unwrap().len(), 1);
            assert_eq!(inner.finalized.get(&block_hash1).unwrap().len(), 1);
        }

        tokio::time::delay_for(DEPLOY_BUFFER_PRUNE_INTERVAL + Duration::from_secs(1)).await;

        {
            let inner = buffer.inner.lock().unwrap();
            assert_eq!(inner.collected_deploys.len(), 0);
            assert_eq!(inner.processed.get(&block_hash2), None);
            assert_eq!(inner.finalized.get(&block_hash1), None);
        }
    }

    #[tokio::test]
    async fn test_deploy_dependencies() {
        let creation_time = Timestamp::from(100);
        let ttl = TimeDiff::from(100);
        let block_time = Timestamp::from(120);

        let mut rng = TestRng::new();
        let (hash1, deploy1) = generate_deploy(&mut rng, creation_time, ttl, vec![]);
        // let deploy2 depend on deploy1
        let (hash2, deploy2) = generate_deploy(&mut rng, creation_time, ttl, vec![hash1]);

        let mut blocks = HashSet::new();
        let mut buffer = DeployBuffer::new(NodeConfig::default().block_max_deploy_count as usize);

        // add deploy2
        buffer.add_deploy(hash2, deploy2);

        // deploy2 has an unsatisfied dependency
        assert!(buffer
            .remaining_deploys(DeployConfig::default(), block_time, blocks.clone())
            .is_empty());

        // add deploy1
        buffer.add_deploy(hash1, deploy1);

        let deploys = buffer.remaining_deploys(DeployConfig::default(), block_time, blocks.clone());
        // only deploy1 should be returned, as it has no dependencies
        assert_eq!(deploys.len(), 1);
        assert!(deploys.contains(&hash1));

        // the deploy will be included in block 1
        let block_hash1 = ProtoBlockHash::new(hash(random::<[u8; 16]>()));
        buffer.added_block(block_hash1, deploys);
        blocks.insert(block_hash1);

        let deploys2 = buffer.remaining_deploys(DeployConfig::default(), block_time, blocks);
        // `blocks` contains a block that contains deploy1 now, so we should get deploy2
        assert_eq!(deploys2.len(), 1);
        assert!(deploys2.contains(&hash2));
    }
}
