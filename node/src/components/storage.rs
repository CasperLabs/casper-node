mod block_height_store;
mod chainspec_store;
mod config;
mod error;
mod event;
mod in_mem_block_height_store;
mod in_mem_chainspec_store;
mod in_mem_store;
mod lmdb_block_height_store;
mod lmdb_chainspec_store;
mod lmdb_store;
mod store;

use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
    fs,
    hash::Hash,
    sync::Arc,
};

use datasize::DataSize;
use futures::TryFutureExt;
use semver::Version;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use tokio::task;
use tracing::{debug, error, warn};

use crate::{
    components::{
        block_proposer::BlockProposerState, chainspec_loader::Chainspec, small_network::NodeId,
        Component,
    },
    crypto::asymmetric_key::Signature,
    effect::{
        requests::{NetworkRequest, StorageRequest},
        EffectBuilder, EffectExt, Effects, Responder,
    },
    protocol::Message,
    types::{
        json_compatibility::ExecutionResult, Block, CryptoRngCore, Deploy, DeployHash,
        DeployHeader, Item, ProtoBlockHash, Timestamp,
    },
    utils::WithDir,
};
use block_height_store::BlockHeightStore;
use chainspec_store::ChainspecStore;
pub use config::Config;
pub use error::Error;
pub(crate) use error::Result;
pub use event::Event;
use in_mem_block_height_store::InMemBlockHeightStore;
use in_mem_chainspec_store::InMemChainspecStore;
use in_mem_store::InMemStore;
use lmdb_block_height_store::LmdbBlockHeightStore;
use lmdb_chainspec_store::LmdbChainspecStore;
use lmdb_store::LmdbStore;
use store::{DeployStore, Multiple, Store};

pub(crate) type Storage = LmdbStorage<Block, Deploy>;

pub(crate) type DeployResults<S> = Multiple<Option<<S as StorageType>::Deploy>>;
pub(crate) type DeployHashes<S> = Multiple<<<S as StorageType>::Deploy as Value>::Id>;
pub(crate) type DeployHeaderResults<S> =
    Multiple<Option<<<S as StorageType>::Deploy as Value>::Header>>;
type DeployAndMetadata<D, B> = (D, DeployMetadata<B>);

const BLOCK_STORE_FILENAME: &str = "block_store.db";
const BLOCK_HEIGHT_STORE_FILENAME: &str = "block_height_store.db";
const DEPLOY_STORE_FILENAME: &str = "deploy_store.db";
const CHAINSPEC_STORE_FILENAME: &str = "chainspec_store.db";

pub trait ValueT: Clone + Serialize + DeserializeOwned + Send + Sync + Debug + Display {}
impl<T> ValueT for T where T: Clone + Serialize + DeserializeOwned + Send + Sync + Debug + Display {}

/// Trait defining the API for a value able to be held within the storage component.
pub trait Value: ValueT {
    type Id: Copy
        + Clone
        + Ord
        + PartialOrd
        + Eq
        + PartialEq
        + Hash
        + Debug
        + Display
        + Serialize
        + DeserializeOwned
        + Send
        + Sync;
    /// A relatively small portion of the value, representing header info or metadata.
    type Header: Clone
        + Ord
        + PartialOrd
        + Eq
        + PartialEq
        + Hash
        + Debug
        + Display
        + Serialize
        + DeserializeOwned
        + Send
        + Sync;

    fn id(&self) -> &Self::Id;
    fn header(&self) -> &Self::Header;
    fn take_header(self) -> Self::Header;
}

pub trait WithBlockHeight: Value {
    fn height(&self) -> u64;
}

/// Metadata associated with a block.
#[derive(Default, Clone, Serialize, Deserialize, Debug)]
pub struct BlockMetadata {
    /// The finalization signatures of a block.
    pub proofs: Vec<Signature>,
}

/// Metadata associated with a deploy.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DeployMetadata<B: Value> {
    /// The block hashes of blocks containing the related deploy, along with the results of
    /// executing the related deploy.
    pub execution_results: HashMap<B::Id, ExecutionResult>,
}

impl<B: Value> DeployMetadata<B> {
    fn new(block_hash: B::Id, execution_result: ExecutionResult) -> Self {
        let mut execution_results = HashMap::new();
        let _ = execution_results.insert(block_hash, execution_result);
        DeployMetadata { execution_results }
    }
}

impl<B: Value> Default for DeployMetadata<B> {
    fn default() -> Self {
        DeployMetadata {
            execution_results: HashMap::new(),
        }
    }
}

impl LmdbStorage<Block, Deploy> {
    async fn load_block_deploys(&self, block: &Block) -> (ProtoBlockHash, Vec<Deploy>) {
        let deploy_store = self.deploy_store();
        let deploy_hashes = SmallVec::from(block.deploy_hashes().clone());
        let block_hash = ProtoBlockHash::from_parts(&deploy_hashes, block.header().random_bit());
        let deploys = task::spawn_blocking(move || deploy_store.get(deploy_hashes))
            .await
            .expect("should run")
            .into_iter()
            .map(|result| result.unwrap_or_else(|error| panic!("failed to get deploy: {}", error)))
            .flatten()
            .collect::<Vec<_>>();
        (block_hash, deploys)
    }

    fn load_pending_deploys(
        &self,
        finalized: &HashSet<DeployHash>,
        current_instant: Timestamp,
    ) -> Result<HashMap<DeployHash, DeployHeader>> {
        let ids = self.deploy_store().ids()?;
        let mut pending = HashMap::new();
        for id in ids {
            let deploy = self
                .deploy_store()
                .get(smallvec![id])
                .pop()
                .expect("should pop")
                .expect("should load")
                .expect("should be some");

            let header = deploy.header();
            if header.expired(current_instant) {
                break;
            }
            if !finalized.contains(deploy.id()) {
                pending.insert(*deploy.id(), header.clone());
            }
        }
        Ok(pending)
    }

    /// This method is intended to only be used by the joiner when transitioning to the validator
    /// state.
    pub(crate) async fn load_block_proposer_state(
        &self,
        latest_block_height: u64,
        chainspec_version: Version,
        current_instant: Timestamp,
    ) -> BlockProposerState {
        let max_ttl = {
            let chainspec_store = self.chainspec_store();
            let chainspec = task::spawn_blocking(move || chainspec_store.get(chainspec_version))
                .await
                .expect("should run blocking");
            let chainspec = match chainspec {
                Ok(Some(chainspec)) => chainspec,
                // If we can't get our hands on a chainspec, then we can't get a max_ttl to compare
                // blocks and deploys against.
                _ => panic!("unable to load chainspec"),
            };
            chainspec.genesis.deploy_config.max_ttl
        };

        // deploys, organized by ProtoBlockHash, which have been finalized
        let mut finalized = HashMap::new();
        let mut finalized_hashes = HashSet::new();

        'iterate_ancestry: for height in (0..=latest_block_height).rev() {
            let block = {
                let block_store = self.block_store();
                let block_by_height_store = self.block_height_store();

                let ancestor_hash = task::spawn_blocking(move || block_by_height_store.get(height))
                    .await
                    .expect("should spawn_blocking");

                let ancestor_hash = match ancestor_hash {
                    Ok(Some(hash)) => hash,
                    _ => break 'iterate_ancestry,
                };

                task::spawn_blocking(move || block_store.get(smallvec![ancestor_hash]))
                    .await
                    .expect("should spawn_blocking")
                    .pop()
                    .expect("should pop")
                    .expect("should load")
                    .unwrap_or_else(|| panic!("block at height {} should exist", height))
            };

            if block.header().timestamp() < current_instant - max_ttl {
                break 'iterate_ancestry;
            }

            let (block_hash, deploys) = self.load_block_deploys(&block).await;
            let deploys = deploys
                .iter()
                .map(|deploy| (*deploy.id(), deploy.header().clone()))
                .collect::<HashMap<_, _>>();

            finalized_hashes.extend(deploys.iter().map(|(hash, _)| hash));
            finalized.insert(block_hash, deploys);
        }

        // Once finalized block's deploys are loaded, iterate over Deploy store to find 'pending'
        // deploys.
        let pending = self
            .load_pending_deploys(&finalized_hashes, current_instant)
            .expect("should load pending deploys");

        BlockProposerState::with_pending_and_finalized(pending, finalized)
    }
}

/// Trait which will handle management of the various storage sub-components.
///
/// If this trait is ultimately only used for testing scenarios, we shouldn't need to expose it to
/// the reactor - it can simply use a concrete type which implements this trait.
pub trait StorageType {
    type Block: Value + WithBlockHeight;
    type Deploy: Value + Item;

    fn block_store(&self) -> Arc<dyn Store<Value = Self::Block>>;

    fn block_height_store(&self) -> Arc<dyn BlockHeightStore<<Self::Block as Value>::Id>>;

    fn deploy_store(
        &self,
    ) -> Arc<dyn DeployStore<Block = Self::Block, Deploy = Self::Deploy, Value = Self::Deploy>>;

    fn chainspec_store(&self) -> Arc<dyn ChainspecStore>;

    fn new(config: WithDir<Config>) -> Result<Self>
    where
        Self: Sized;

    fn get_deploy_for_peer<REv>(
        &self,
        effect_builder: EffectBuilder<REv>,
        deploy_hash: <Self::Deploy as Value>::Id,
        peer: NodeId,
    ) -> Effects<Event<Self>>
    where
        REv: From<NetworkRequest<NodeId, Message>> + Send,
        Self: Sized,
    {
        let deploy_store = self.deploy_store();
        let deploy_hashes = smallvec![deploy_hash];
        async move {
            task::spawn_blocking(move || deploy_store.get(deploy_hashes))
                .await
                .expect("should run")
                .pop()
                .expect("can only contain one result")
        }
        .map_err(move |error| debug!("failed to get {} for {}: {}", deploy_hash, peer, error))
        .and_then(move |maybe_deploy| async move {
            match maybe_deploy {
                Some(deploy) => match Message::new_get_response(&deploy) {
                    Ok(message) => effect_builder.send_message(peer, message).await,
                    Err(error) => error!("failed to create get-response: {}", error),
                },
                None => debug!("failed to get {} for {}", deploy_hash, peer),
            }
            Ok(())
        })
        .ignore()
    }

    fn put_block(&self, block: Box<Self::Block>, responder: Responder<bool>) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let block_store = self.block_store();
        let block_height_store = self.block_height_store();
        async move {
            let result = task::spawn_blocking(move || {
                let height = block.height();
                let block_hash = *block.id();
                let height_result =
                    block_height_store
                        .put(height, block_hash)
                        .unwrap_or_else(|error| {
                            panic!("failed to put height for {}: {}", block_hash, error)
                        });
                let block_result = block_store
                    .put(*block)
                    .unwrap_or_else(|error| panic!("failed to put {}: {}", block_hash, error));
                // TODO: once blocks' signatures are handled as metadata, this condition can be
                //       changed to just `height_result != block_result`.
                if height_result != block_result && !block_result {
                    panic!(
                        "mismatch in put results. height_result: {}. block_result: {}",
                        height_result, block_result
                    );
                }
                height_result
            })
            .await
            .expect("should run");
            responder.respond(result).await
        }
        .ignore()
    }

    fn get_block(
        &self,
        block_hash: <Self::Block as Value>::Id,
        responder: Responder<Option<Self::Block>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let block_store = self.block_store();
        async move {
            let mut results = task::spawn_blocking(move || block_store.get(smallvec![block_hash]))
                .await
                .expect("should run");
            let result = results
                .pop()
                .expect("can only contain one result")
                .unwrap_or_else(|error| panic!("failed to get {}: {}", block_hash, error));
            responder.respond(result).await
        }
        .ignore()
    }

    fn get_block_at_height(
        &self,
        block_height: u64,
        responder: Responder<Option<Self::Block>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let block_height_store = self.block_height_store();
        let block_store = self.block_store();
        async move {
            let result = task::spawn_blocking(move || {
                block_height_store
                    .get(block_height)
                    .unwrap_or_else(|error| {
                        panic!(
                            "failed to get entry for block height {}: {}",
                            block_height, error
                        )
                    })
                    .and_then(|block_hash| {
                        block_store
                            .get(smallvec![block_hash])
                            .pop()
                            .expect("can only contain one result")
                            .unwrap_or_else(|error| {
                                panic!("failed to get block {}: {}", block_hash, error)
                            })
                    })
            })
            .await
            .expect("should run");
            responder.respond(result).await
        }
        .ignore()
    }

    fn get_highest_block(&self, responder: Responder<Option<Self::Block>>) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let block_height_store = self.block_height_store();
        let block_store = self.block_store();
        async move {
            let result = task::spawn_blocking(move || {
                block_height_store
                    .highest()
                    .unwrap_or_else(|error| {
                        panic!("failed to get entry for latest block: {}", error)
                    })
                    .and_then(|block_hash| {
                        block_store
                            .get(smallvec![block_hash])
                            .pop()
                            .expect("can only contain one result")
                            .unwrap_or_else(|error| {
                                panic!("failed to get block {}: {}", block_hash, error)
                            })
                    })
            })
            .await
            .expect("should run");
            responder.respond(result).await
        }
        .ignore()
    }

    fn get_block_header(
        &self,
        block_hash: <Self::Block as Value>::Id,
        responder: Responder<Option<<Self::Block as Value>::Header>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let block_store = self.block_store();
        async move {
            let mut results =
                task::spawn_blocking(move || block_store.get_headers(smallvec![block_hash]))
                    .await
                    .expect("should run");
            let result = results
                .pop()
                .expect("can only contain one result")
                .unwrap_or_else(|error| panic!("failed to get header {}: {}", block_hash, error));
            responder.respond(result).await
        }
        .ignore()
    }

    fn put_deploy(
        &self,
        deploy: Box<Self::Deploy>,
        responder: Responder<bool>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let deploy_store = self.deploy_store();
        let deploy_hash = *Value::id(&*deploy);
        async move {
            let result = task::spawn_blocking(move || deploy_store.put(*deploy))
                .await
                .expect("should run")
                .unwrap_or_else(|error| panic!("failed to put {}: {}", deploy_hash, error));
            responder.respond(result).await;
        }
        .ignore()
    }

    fn get_deploys(
        &self,
        deploy_hashes: DeployHashes<Self>,
        responder: Responder<DeployResults<Self>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let deploy_store = self.deploy_store();
        async move {
            let results = task::spawn_blocking(move || deploy_store.get(deploy_hashes))
                .await
                .expect("should run")
                .into_iter()
                .map(|result| {
                    result.unwrap_or_else(|error| panic!("failed to get deploy: {}", error))
                })
                .collect();
            responder.respond(results).await
        }
        .ignore()
    }

    fn get_deploy_headers(
        &self,
        deploy_hashes: DeployHashes<Self>,
        responder: Responder<DeployHeaderResults<Self>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let deploy_store = self.deploy_store();
        async move {
            let results = task::spawn_blocking(move || deploy_store.get_headers(deploy_hashes))
                .await
                .expect("should run")
                .into_iter()
                .map(|result| {
                    result.unwrap_or_else(|error| panic!("failed to get deploy header: {}", error))
                })
                .collect();
            responder.respond(results).await
        }
        .ignore()
    }

    fn put_execution_results(
        &self,
        block_hash: <Self::Block as Value>::Id,
        execution_results: HashMap<<Self::Deploy as Value>::Id, ExecutionResult>,
        responder: Responder<()>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let deploy_store = self.deploy_store();
        async move {
            task::spawn_blocking(move || {
                for (deploy_hash, execution_result) in execution_results.into_iter() {
                    match deploy_store.put_execution_result(
                        deploy_hash,
                        block_hash,
                        execution_result,
                    ) {
                        Ok(true) => (),
                        Ok(false) => {
                            warn!(%deploy_hash, %block_hash, "already stored execution result")
                        }
                        Err(error) => panic!(
                            "failed to put execution results {} {}: {}",
                            deploy_hash, block_hash, error
                        ),
                    }
                }
            })
            .await
            .expect("should run");
            responder.respond(()).await
        }
        .ignore()
    }

    fn get_deploy_and_metadata(
        &self,
        deploy_hash: <Self::Deploy as Value>::Id,
        responder: Responder<Option<DeployAndMetadata<Self::Deploy, Self::Block>>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let deploy_store = self.deploy_store();
        async move {
            let result =
                task::spawn_blocking(move || deploy_store.get_deploy_and_metadata(deploy_hash))
                    .await
                    .expect("should run")
                    .unwrap_or_else(|error| panic!("failed to get deploy and metadata: {}", error));
            responder.respond(result).await
        }
        .ignore()
    }

    fn put_chainspec(
        &self,
        chainspec: Box<Chainspec>,
        responder: Responder<()>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let chainspec_store = self.chainspec_store();
        async move {
            task::spawn_blocking(move || chainspec_store.put(*chainspec))
                .await
                .expect("should run")
                .unwrap_or_else(|error| panic!("failed to put chainspec: {}", error));
            responder.respond(()).await
        }
        .ignore()
    }

    fn get_chainspec(
        &self,
        version: Version,
        responder: Responder<Option<Chainspec>>,
    ) -> Effects<Event<Self>>
    where
        Self: Sized,
    {
        let chainspec_store = self.chainspec_store();
        async move {
            let result = task::spawn_blocking(move || chainspec_store.get(version))
                .await
                .expect("should run")
                .unwrap_or_else(|error| panic!("failed to get chainspec: {}", error));
            responder.respond(result).await
        }
        .ignore()
    }
}

impl<REv, S> Component<REv> for S
where
    REv: From<NetworkRequest<NodeId, Message>> + Send,
    S: StorageType,
    Self: Sized + 'static,
{
    type Event = Event<S>;
    type ConstructionError = Error;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut dyn CryptoRngCore,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::GetDeployForPeer { deploy_hash, peer } => {
                self.get_deploy_for_peer(effect_builder, deploy_hash, peer)
            }
            Event::Request(StorageRequest::PutBlock { block, responder }) => {
                self.put_block(block, responder)
            }
            Event::Request(StorageRequest::GetBlock {
                block_hash,
                responder,
            }) => self.get_block(block_hash, responder),
            Event::Request(StorageRequest::GetBlockAtHeight { height, responder }) => {
                self.get_block_at_height(height, responder)
            }
            Event::Request(StorageRequest::GetHighestBlock { responder }) => {
                self.get_highest_block(responder)
            }
            Event::Request(StorageRequest::GetBlockHeader {
                block_hash,
                responder,
            }) => self.get_block_header(block_hash, responder),
            Event::Request(StorageRequest::PutDeploy { deploy, responder }) => {
                self.put_deploy(deploy, responder)
            }
            Event::Request(StorageRequest::GetDeploys {
                deploy_hashes,
                responder,
            }) => self.get_deploys(deploy_hashes, responder),
            Event::Request(StorageRequest::GetDeployHeaders {
                deploy_hashes,
                responder,
            }) => self.get_deploy_headers(deploy_hashes, responder),
            Event::Request(StorageRequest::PutExecutionResults {
                block_hash,
                execution_results,
                responder,
            }) => self.put_execution_results(block_hash, execution_results, responder),
            Event::Request(StorageRequest::GetDeployAndMetadata {
                deploy_hash,
                responder,
            }) => self.get_deploy_and_metadata(deploy_hash, responder),
            Event::Request(StorageRequest::PutChainspec {
                chainspec,
                responder,
            }) => self.put_chainspec(chainspec, responder),
            Event::Request(StorageRequest::GetChainspec { version, responder }) => {
                self.get_chainspec(version, responder)
            }
        }
    }
}

// Concrete type of `Storage` backed by in-memory stores.
#[derive(Debug)]
pub(crate) struct InMemStorage<B: Value, D: Value> {
    block_store: Arc<InMemStore<B, BlockMetadata>>,
    block_height_store: Arc<InMemBlockHeightStore<B::Id>>,
    deploy_store: Arc<InMemStore<D, DeployMetadata<B>>>,
    chainspec_store: Arc<InMemChainspecStore>,
}

#[allow(trivial_casts)]
impl<B, D> StorageType for InMemStorage<B, D>
where
    B: Value + WithBlockHeight + 'static,
    D: Value + Item + 'static,
{
    type Block = B;
    type Deploy = D;

    fn block_store(&self) -> Arc<dyn Store<Value = B>> {
        Arc::clone(&self.block_store) as Arc<dyn Store<Value = B>>
    }

    fn block_height_store(&self) -> Arc<dyn BlockHeightStore<B::Id>> {
        Arc::clone(&self.block_height_store) as Arc<dyn BlockHeightStore<B::Id>>
    }

    fn deploy_store(&self) -> Arc<dyn DeployStore<Block = B, Deploy = D, Value = D>> {
        Arc::clone(&self.deploy_store) as Arc<dyn DeployStore<Block = B, Deploy = D, Value = D>>
    }

    fn chainspec_store(&self) -> Arc<dyn ChainspecStore> {
        Arc::clone(&self.chainspec_store) as Arc<dyn ChainspecStore>
    }

    fn new(_config: WithDir<Config>) -> Result<Self> {
        Ok(InMemStorage {
            block_store: Arc::new(InMemStore::new()),
            block_height_store: Arc::new(InMemBlockHeightStore::new()),
            deploy_store: Arc::new(InMemStore::new()),
            chainspec_store: Arc::new(InMemChainspecStore::new()),
        })
    }
}

// Concrete type of `Storage` backed by LMDB stores.
#[derive(DataSize, Debug)]
pub struct LmdbStorage<B, D>
where
    B: Value,
    D: Value,
{
    block_store: Arc<LmdbStore<B, BlockMetadata>>,
    block_height_store: Arc<LmdbBlockHeightStore>,
    deploy_store: Arc<LmdbStore<D, DeployMetadata<B>>>,
    chainspec_store: Arc<LmdbChainspecStore>,
}

#[allow(trivial_casts)]
impl<B, D> StorageType for LmdbStorage<B, D>
where
    B: Value + WithBlockHeight + 'static,
    D: Value + Item + 'static,
{
    type Block = B;
    type Deploy = D;

    fn new(config: WithDir<Config>) -> Result<Self> {
        let root = config.with_dir(config.value().path());
        fs::create_dir_all(&root).map_err(|error| Error::CreateDir {
            dir: root.display().to_string(),
            source: error,
        })?;

        let block_store_path = root.join(BLOCK_STORE_FILENAME);
        let block_height_store_path = root.join(BLOCK_HEIGHT_STORE_FILENAME);
        let deploy_store_path = root.join(DEPLOY_STORE_FILENAME);
        let chainspec_store_path = root.join(CHAINSPEC_STORE_FILENAME);

        let block_store = LmdbStore::new(block_store_path, config.value().max_block_store_size())?;
        let block_height_store = LmdbBlockHeightStore::new(
            block_height_store_path,
            config.value().max_block_height_store_size(),
        )?;
        let deploy_store =
            LmdbStore::new(deploy_store_path, config.value().max_deploy_store_size())?;
        let chainspec_store = LmdbChainspecStore::new(
            chainspec_store_path,
            config.value().max_chainspec_store_size(),
        )?;

        Ok(LmdbStorage {
            block_store: Arc::new(block_store),
            block_height_store: Arc::new(block_height_store),
            deploy_store: Arc::new(deploy_store),
            chainspec_store: Arc::new(chainspec_store),
        })
    }

    fn block_store(&self) -> Arc<dyn Store<Value = B>> {
        Arc::clone(&self.block_store) as Arc<dyn Store<Value = B>>
    }

    fn block_height_store(&self) -> Arc<dyn BlockHeightStore<B::Id>> {
        Arc::clone(&self.block_height_store) as Arc<dyn BlockHeightStore<B::Id>>
    }

    fn deploy_store(&self) -> Arc<dyn DeployStore<Block = B, Deploy = D, Value = D>> {
        Arc::clone(&self.deploy_store) as Arc<dyn DeployStore<Block = B, Deploy = D, Value = D>>
    }

    fn chainspec_store(&self) -> Arc<dyn ChainspecStore> {
        Arc::clone(&self.chainspec_store) as Arc<dyn ChainspecStore>
    }
}
