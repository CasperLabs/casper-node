use std::{
    collections::BTreeMap,
    convert::{TryFrom, TryInto},
    ffi::OsStr,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use lmdb::DatabaseFlags;
use log::LevelFilter;

use bytesrepr::FromBytes;
use casper_execution_engine::{
    core::{
        engine_state,
        engine_state::{
            era_validators::GetEraValidatorsRequest,
            execute_request::ExecuteRequest,
            execution_result::ExecutionResult,
            run_genesis_request::RunGenesisRequest,
            step::{StepRequest, StepResult},
            BalanceResult, EngineConfig, EngineState, GenesisResult, GetBidsRequest, QueryRequest,
            QueryResult, UpgradeConfig, UpgradeResult,
        },
        execution,
    },
    shared::{
        account::Account,
        additive_map::AdditiveMap,
        gas::Gas,
        logging::{self, Settings, Style},
        newtypes::{Blake2bHash, CorrelationId},
        stored_value::StoredValue,
        transform::Transform,
        utils::OS_PAGE_SIZE,
    },
    storage::{
        global_state::{
            in_memory::InMemoryGlobalState, lmdb::LmdbGlobalState, CommitResult, StateProvider,
            StateReader,
        },
        protocol_data_store::lmdb::LmdbProtocolDataStore,
        transaction_source::lmdb::LmdbEnvironment,
        trie::merkle_proof::TrieMerkleProof,
        trie_store::lmdb::LmdbTrieStore,
    },
};
use casper_types::{
    account::AccountHash,
    bytesrepr::{self},
    runtime_args,
    system::{
        auction::{
            Bids, EraValidators, UnbondingPurses, ValidatorWeights, ARG_ERA_END_TIMESTAMP_MILLIS,
            ARG_EVICTED_VALIDATORS, AUCTION_DELAY_KEY, ERA_ID_KEY, METHOD_RUN_AUCTION,
        },
        mint::TOTAL_SUPPLY_KEY,
    },
    CLTyped, CLValue, Contract, ContractHash, ContractPackage, ContractPackageHash, ContractWasm,
    DeployHash, DeployInfo, EraId, Key, KeyTag, PublicKey, RuntimeArgs, Transfer, TransferAddr,
    URef, U512,
};

use crate::internal::{
    utils, ExecuteRequestBuilder, DEFAULT_PROPOSER_ADDR, DEFAULT_PROTOCOL_VERSION, SYSTEM_ADDR,
};

/// LMDB initial map size is calculated based on DEFAULT_LMDB_PAGES and systems page size.
const DEFAULT_LMDB_PAGES: usize = 256_000_000;

/// LMDB max readers
///
/// The default value is chosen to be the same as the node itself.
const DEFAULT_MAX_READERS: u32 = 512;

/// This is appended to the data dir path provided to the `LmdbWasmTestBuilder`".
const GLOBAL_STATE_DIR: &str = "global_state";

pub type InMemoryWasmTestBuilder = WasmTestBuilder<InMemoryGlobalState>;
pub type LmdbWasmTestBuilder = WasmTestBuilder<LmdbGlobalState>;

/// Builder for simple WASM test
pub struct WasmTestBuilder<S> {
    /// [`EngineState`] is wrapped in [`Rc`] to work around a missing [`Clone`] implementation
    engine_state: Rc<EngineState<S>>,
    /// [`ExecutionResult`] is wrapped in [`Rc`] to work around a missing [`Clone`] implementation
    exec_results: Vec<Vec<Rc<ExecutionResult>>>,
    upgrade_results: Vec<Result<UpgradeResult, engine_state::Error>>,
    genesis_hash: Option<Blake2bHash>,
    post_state_hash: Option<Blake2bHash>,
    /// Cached transform maps after subsequent successful runs i.e. `transforms[0]` is for first
    /// exec call etc.
    transforms: Vec<AdditiveMap<Key, Transform>>,
    /// Cached genesis transforms
    genesis_account: Option<Account>,
    /// Genesis transforms
    genesis_transforms: Option<AdditiveMap<Key, Transform>>,
    /// Mint contract key
    mint_contract_hash: Option<ContractHash>,
    /// Handle payment contract key
    handle_payment_contract_hash: Option<ContractHash>,
    /// Standard payment contract key
    standard_payment_hash: Option<ContractHash>,
    /// Auction contract key
    auction_contract_hash: Option<ContractHash>,
}

impl<S> WasmTestBuilder<S> {
    fn initialize_logging() {
        let log_settings = Settings::new(LevelFilter::Error).with_style(Style::HumanReadable);
        let _ = logging::initialize(log_settings);
    }
}

impl Default for InMemoryWasmTestBuilder {
    fn default() -> Self {
        Self::initialize_logging();
        let engine_config = EngineConfig::default();

        let global_state = InMemoryGlobalState::empty().expect("should create global state");
        let engine_state = EngineState::new(global_state, engine_config);

        WasmTestBuilder {
            engine_state: Rc::new(engine_state),
            exec_results: Vec::new(),
            upgrade_results: Vec::new(),
            genesis_hash: None,
            post_state_hash: None,
            transforms: Vec::new(),
            genesis_account: None,
            genesis_transforms: None,
            mint_contract_hash: None,
            handle_payment_contract_hash: None,
            standard_payment_hash: None,
            auction_contract_hash: None,
        }
    }
}

// TODO: Deriving `Clone` for `WasmTestBuilder<S>` doesn't work correctly (unsure why), so
// implemented by hand here.  Try to derive in the future with a different compiler version.
impl<S> Clone for WasmTestBuilder<S> {
    fn clone(&self) -> Self {
        WasmTestBuilder {
            engine_state: Rc::clone(&self.engine_state),
            exec_results: self.exec_results.clone(),
            upgrade_results: self.upgrade_results.clone(),
            genesis_hash: self.genesis_hash,
            post_state_hash: self.post_state_hash,
            transforms: self.transforms.clone(),
            genesis_account: self.genesis_account.clone(),
            genesis_transforms: self.genesis_transforms.clone(),
            mint_contract_hash: self.mint_contract_hash,
            handle_payment_contract_hash: self.handle_payment_contract_hash,
            standard_payment_hash: self.standard_payment_hash,
            auction_contract_hash: self.auction_contract_hash,
        }
    }
}

/// A wrapper type to disambiguate builder from an actual result
#[derive(Clone)]
pub struct WasmTestResult<S>(WasmTestBuilder<S>);

impl<S> WasmTestResult<S> {
    /// Access the builder
    pub fn builder(&self) -> &WasmTestBuilder<S> {
        &self.0
    }
}

impl InMemoryWasmTestBuilder {
    pub fn new(
        global_state: InMemoryGlobalState,
        engine_config: EngineConfig,
        post_state_hash: Blake2bHash,
    ) -> Self {
        Self::initialize_logging();
        let engine_state = EngineState::new(global_state, engine_config);
        WasmTestBuilder {
            engine_state: Rc::new(engine_state),
            genesis_hash: Some(post_state_hash),
            post_state_hash: Some(post_state_hash),
            ..Default::default()
        }
    }
}

impl LmdbWasmTestBuilder {
    pub fn new_with_config<T: AsRef<OsStr> + ?Sized>(
        data_dir: &T,
        engine_config: EngineConfig,
    ) -> Self {
        Self::initialize_logging();
        let page_size = *OS_PAGE_SIZE;
        let global_state_dir = Self::global_state_dir(data_dir);
        Self::create_global_state_dir(&global_state_dir);
        let environment = Arc::new(
            LmdbEnvironment::new(
                &global_state_dir,
                page_size * DEFAULT_LMDB_PAGES,
                DEFAULT_MAX_READERS,
            )
            .expect("should create LmdbEnvironment"),
        );
        let trie_store = Arc::new(
            LmdbTrieStore::new(&environment, None, DatabaseFlags::empty())
                .expect("should create LmdbTrieStore"),
        );
        let protocol_data_store = Arc::new(
            LmdbProtocolDataStore::new(&environment, None, DatabaseFlags::empty())
                .expect("should create LmdbProtocolDataStore"),
        );
        let global_state = LmdbGlobalState::empty(environment, trie_store, protocol_data_store)
            .expect("should create LmdbGlobalState");
        let engine_state = EngineState::new(global_state, engine_config);
        WasmTestBuilder {
            engine_state: Rc::new(engine_state),
            exec_results: Vec::new(),
            upgrade_results: Vec::new(),
            genesis_hash: None,
            post_state_hash: None,
            transforms: Vec::new(),
            genesis_account: None,
            genesis_transforms: None,
            mint_contract_hash: None,
            handle_payment_contract_hash: None,
            standard_payment_hash: None,
            auction_contract_hash: None,
        }
    }

    pub fn flush_environment(&self) {
        let engine_state = &*self.engine_state;
        engine_state.flush_environment();
    }

    pub fn new<T: AsRef<OsStr> + ?Sized>(data_dir: &T) -> Self {
        Self::new_with_config(data_dir, Default::default())
    }

    /// Creates new instance of builder and applies values only which allows the engine state to be
    /// swapped with a new one, possibly after running genesis once and reusing existing database
    /// (i.e. LMDB).
    pub fn new_with_config_and_result<T: AsRef<OsStr> + ?Sized>(
        data_dir: &T,
        engine_config: EngineConfig,
        result: &WasmTestResult<LmdbGlobalState>,
    ) -> Self {
        let mut builder = Self::new_with_config(data_dir, engine_config);
        // Applies existing properties from gi
        builder.genesis_hash = result.0.genesis_hash;
        builder.post_state_hash = result.0.post_state_hash;
        builder.mint_contract_hash = result.0.mint_contract_hash;
        builder.handle_payment_contract_hash = result.0.handle_payment_contract_hash;
        builder
    }

    /// Creates a new instance of builder using the supplied configurations, opening wrapped LMDBs
    /// (e.g. in the Trie and Data stores) rather than creating them.
    pub fn open<T: AsRef<OsStr> + ?Sized>(
        data_dir: &T,
        engine_config: EngineConfig,
        post_state_hash: Blake2bHash,
    ) -> Self {
        let global_state_path = Self::global_state_dir(data_dir);
        Self::open_raw(global_state_path, engine_config, post_state_hash)
    }

    /// Creates a new instance of builder using the supplied configurations, opening wrapped LMDBs
    /// (e.g. in the Trie and Data stores) rather than creating them.
    /// Differs from `open` in that it doesn't append `GLOBAL_STATE_DIR` to the supplied path.
    pub fn open_raw<T: AsRef<Path>>(
        global_state_dir: T,
        engine_config: EngineConfig,
        post_state_hash: Blake2bHash,
    ) -> Self {
        Self::initialize_logging();
        let page_size = *OS_PAGE_SIZE;
        Self::create_global_state_dir(&global_state_dir);
        let environment = Arc::new(
            LmdbEnvironment::new(
                &global_state_dir,
                page_size * DEFAULT_LMDB_PAGES,
                DEFAULT_MAX_READERS,
            )
            .expect("should create LmdbEnvironment"),
        );
        let trie_store =
            Arc::new(LmdbTrieStore::open(&environment, None).expect("should open LmdbTrieStore"));
        let protocol_data_store = Arc::new(
            LmdbProtocolDataStore::open(&environment, None)
                .expect("should open LmdbProtocolDataStore"),
        );
        let global_state = LmdbGlobalState::empty(environment, trie_store, protocol_data_store)
            .expect("should create LmdbGlobalState");
        let engine_state = EngineState::new(global_state, engine_config);
        WasmTestBuilder {
            engine_state: Rc::new(engine_state),
            exec_results: Vec::new(),
            upgrade_results: Vec::new(),
            genesis_hash: None,
            post_state_hash: Some(post_state_hash),
            transforms: Vec::new(),
            genesis_account: None,
            genesis_transforms: None,
            mint_contract_hash: None,
            handle_payment_contract_hash: None,
            standard_payment_hash: None,
            auction_contract_hash: None,
        }
    }

    fn create_global_state_dir<T: AsRef<Path>>(global_state_path: T) {
        fs::create_dir_all(&global_state_path).unwrap_or_else(|_| {
            panic!(
                "Expected to create {}",
                global_state_path.as_ref().display()
            )
        });
    }

    fn global_state_dir<T: AsRef<OsStr> + ?Sized>(data_dir: &T) -> PathBuf {
        let mut path = PathBuf::from(data_dir);
        path.push(GLOBAL_STATE_DIR);
        path
    }
}

impl<S> WasmTestBuilder<S>
where
    S: StateProvider,
    engine_state::Error: From<S::Error>,
    S::Error: Into<execution::Error>,
{
    /// Carries on attributes from TestResult for further executions
    pub fn from_result(result: WasmTestResult<S>) -> Self {
        WasmTestBuilder {
            engine_state: result.0.engine_state,
            exec_results: Vec::new(),
            upgrade_results: Vec::new(),
            genesis_hash: result.0.genesis_hash,
            post_state_hash: result.0.post_state_hash,
            transforms: Vec::new(),
            genesis_account: result.0.genesis_account,
            mint_contract_hash: result.0.mint_contract_hash,
            handle_payment_contract_hash: result.0.handle_payment_contract_hash,
            standard_payment_hash: result.0.standard_payment_hash,
            auction_contract_hash: result.0.auction_contract_hash,
            genesis_transforms: result.0.genesis_transforms,
        }
    }

    pub fn run_genesis(&mut self, run_genesis_request: &RunGenesisRequest) -> &mut Self {
        let system_account = Key::Account(PublicKey::System.to_account_hash());

        let genesis_result = self
            .engine_state
            .commit_genesis(
                CorrelationId::new(),
                run_genesis_request.genesis_config_hash(),
                run_genesis_request.protocol_version(),
                run_genesis_request.ee_config(),
            )
            .expect("Unable to get genesis response");

        if let GenesisResult::Success {
            post_state_hash,
            effect,
        } = genesis_result
        {
            let state_root_hash = post_state_hash;

            let transforms = effect.transforms;

            let genesis_account = utils::get_account(&transforms, &system_account)
                .expect("Unable to get system account");

            let maybe_protocol_data = self
                .engine_state
                .get_protocol_data(run_genesis_request.protocol_version())
                .expect("should read protocol data");
            let protocol_data = maybe_protocol_data.expect("should have protocol data stored");

            self.genesis_hash = Some(state_root_hash);
            self.post_state_hash = Some(state_root_hash);
            self.mint_contract_hash = Some(protocol_data.mint());
            self.handle_payment_contract_hash = Some(protocol_data.handle_payment());
            self.standard_payment_hash = Some(protocol_data.standard_payment());
            self.auction_contract_hash = Some(protocol_data.auction());
            self.genesis_account = Some(genesis_account);
            self.genesis_transforms = Some(transforms);
            return self;
        }

        panic!("genesis failure: {:?}", genesis_result);
    }

    pub fn query(
        &self,
        maybe_post_state: Option<Blake2bHash>,
        base_key: Key,
        path: &[String],
    ) -> Result<StoredValue, String> {
        let post_state = maybe_post_state
            .or(self.post_state_hash)
            .expect("builder must have a post-state hash");

        let query_request = QueryRequest::new(post_state, base_key, path.to_vec());

        let query_result = self
            .engine_state
            .run_query(CorrelationId::new(), query_request)
            .expect("should get query response");

        if let QueryResult::Success { value, .. } = query_result {
            return Ok(value.deref().clone());
        }

        Err(format!("{:?}", query_result))
    }

    pub fn query_with_proof(
        &self,
        maybe_post_state: Option<Blake2bHash>,
        base_key: Key,
        path: &[String],
    ) -> Result<(StoredValue, Vec<TrieMerkleProof<Key, StoredValue>>), String> {
        let post_state = maybe_post_state
            .or(self.post_state_hash)
            .expect("builder must have a post-state hash");

        let path_vec: Vec<String> = path.to_vec();

        let query_request = QueryRequest::new(post_state, base_key, path_vec);

        let query_result = self
            .engine_state
            .run_query(CorrelationId::new(), query_request)
            .expect("should get query response");

        if let QueryResult::Success { value, proofs } = query_result {
            return Ok((value.deref().clone(), proofs));
        }

        panic! {"{:?}", query_result};
    }

    pub fn total_supply(&self, maybe_post_state: Option<Blake2bHash>) -> U512 {
        let mint_key: Key = self
            .mint_contract_hash
            .expect("should have mint_contract_hash")
            .into();

        let result = self.query(maybe_post_state, mint_key, &[TOTAL_SUPPLY_KEY.to_string()]);

        let total_supply: U512 = if let Ok(StoredValue::CLValue(total_supply)) = result {
            total_supply.into_t().expect("total supply should be U512")
        } else {
            panic!("mint should track total supply");
        };

        total_supply
    }

    pub fn exec(&mut self, mut exec_request: ExecuteRequest) -> &mut Self {
        let exec_request = {
            let hash = self.post_state_hash.expect("expected post_state_hash");
            exec_request.parent_state_hash = hash;
            exec_request
        };
        let maybe_exec_results = self
            .engine_state
            .run_execute(CorrelationId::new(), exec_request);
        assert!(maybe_exec_results.is_ok());
        // Parse deploy results
        let execution_results = maybe_exec_results.as_ref().unwrap();
        // Cache transformations
        self.transforms.extend(
            execution_results
                .iter()
                .map(|res| res.effect().transforms.clone()),
        );
        self.exec_results.push(
            maybe_exec_results
                .unwrap()
                .into_iter()
                .map(Rc::new)
                .collect(),
        );
        self
    }

    /// Commit effects of previous exec call on the latest post-state hash.
    pub fn commit(&mut self) -> &mut Self {
        let prestate_hash = self.post_state_hash.expect("Should have genesis hash");

        let effects = self.transforms.last().cloned().unwrap_or_default();

        self.commit_effects(prestate_hash, effects)
    }

    /// Applies effects to global state.
    pub fn commit_transforms(
        &self,
        pre_state_hash: Blake2bHash,
        effects: AdditiveMap<Key, Transform>,
    ) -> CommitResult {
        self.engine_state
            .apply_effect(CorrelationId::new(), pre_state_hash, effects)
            .expect("should commit")
    }

    /// Runs a commit request, expects a successful response, and
    /// overwrites existing cached post state hash with a new one.
    pub fn commit_effects(
        &mut self,
        prestate_hash: Blake2bHash,
        effects: AdditiveMap<Key, Transform>,
    ) -> &mut Self {
        let commit_result = self.commit_transforms(prestate_hash, effects);

        if let CommitResult::Success { state_root } = commit_result {
            self.post_state_hash = Some(state_root);
            return self;
        }
        panic!(
            "Expected commit success but received a failure instead: {:?}",
            commit_result
        );
    }

    pub fn upgrade_with_upgrade_request(
        &mut self,
        upgrade_config: &mut UpgradeConfig,
    ) -> &mut Self {
        let pre_state_hash = self.post_state_hash.expect("should have state hash");
        upgrade_config.with_pre_state_hash(pre_state_hash);

        let result = self
            .engine_state
            .commit_upgrade(CorrelationId::new(), upgrade_config.clone());

        if let Ok(UpgradeResult::Success {
            post_state_hash, ..
        }) = result
        {
            self.post_state_hash = Some(post_state_hash);
        }

        self.upgrade_results.push(result);
        self
    }

    pub fn run_auction(
        &mut self,
        era_end_timestamp_millis: u64,
        evicted_validators: Vec<PublicKey>,
    ) -> &mut Self {
        let auction = self.get_auction_contract_hash();
        let run_request = ExecuteRequestBuilder::contract_call_by_hash(
            *SYSTEM_ADDR,
            auction,
            METHOD_RUN_AUCTION,
            runtime_args! {
                ARG_ERA_END_TIMESTAMP_MILLIS => era_end_timestamp_millis,
                ARG_EVICTED_VALIDATORS => evicted_validators,
            },
        )
        .build();
        self.exec(run_request).commit().expect_success()
    }

    pub fn step(&mut self, step_request: StepRequest) -> &mut Self {
        let result = self
            .engine_state
            .commit_step(CorrelationId::new(), step_request)
            .expect("should step");

        if let StepResult::Success {
            post_state_hash, ..
        } = result
        {
            self.post_state_hash = Some(post_state_hash);
            self
        } else {
            panic!(
                "Expected successful step result, but instead got error: {:?}",
                result,
            )
        }
    }

    /// Expects a successful run
    pub fn expect_success(&mut self) -> &mut Self {
        // Check first result, as only first result is interesting for a simple test
        let exec_results = self
            .exec_results
            .last()
            .expect("Expected to be called after run()");
        let exec_result = exec_results
            .get(0)
            .expect("Unable to get first deploy result");

        if exec_result.is_failure() {
            panic!(
                "Expected successful execution result, but instead got: {:#?}",
                exec_results,
            );
        }
        self
    }

    /// Expects a failed run
    pub fn expect_failure(&mut self) -> &mut Self {
        // Check first result, as only first result is interesting for a simple test
        let exec_results = self
            .exec_results
            .last()
            .expect("Expected to be called after run()");
        let exec_result = exec_results
            .get(0)
            .expect("Unable to get first deploy result");

        if exec_result.is_success() {
            panic!(
                "Expected failed execution result, but instead got: {:?}",
                exec_results,
            );
        }

        self
    }

    pub fn is_error(&self) -> bool {
        let exec_results = self
            .exec_results
            .last()
            .expect("Expected to be called after run()");
        let exec_result = exec_results
            .get(0)
            .expect("Unable to get first execution result");
        exec_result.is_failure()
    }

    pub fn get_error(&self) -> Option<engine_state::Error> {
        let exec_results = &self.get_exec_results();

        let exec_result = exec_results
            .last()
            .expect("Expected to be called after run()")
            .get(0)
            .expect("Unable to get first deploy result");

        exec_result.as_error().cloned()
    }

    /// Gets the transform map that's cached between runs
    pub fn get_transforms(&self) -> Vec<AdditiveMap<Key, Transform>> {
        self.transforms.clone()
    }

    /// Gets genesis account (if present)
    pub fn get_genesis_account(&self) -> &Account {
        self.genesis_account
            .as_ref()
            .expect("Unable to obtain genesis account. Please run genesis first.")
    }

    pub fn get_mint_contract_hash(&self) -> ContractHash {
        self.mint_contract_hash
            .expect("Unable to obtain mint contract. Please run genesis first.")
    }

    pub fn get_handle_payment_contract_hash(&self) -> ContractHash {
        self.handle_payment_contract_hash
            .expect("Unable to obtain handle payment contract. Please run genesis first.")
    }

    pub fn get_standard_payment_contract_hash(&self) -> ContractHash {
        self.standard_payment_hash
            .expect("Unable to obtain standard payment contract. Please run genesis first.")
    }

    pub fn get_auction_contract_hash(&self) -> ContractHash {
        self.auction_contract_hash
            .expect("Unable to obtain auction contract. Please run genesis first.")
    }

    pub fn get_genesis_transforms(&self) -> &AdditiveMap<Key, Transform> {
        self.genesis_transforms
            .as_ref()
            .expect("should have genesis transforms")
    }

    pub fn get_genesis_hash(&self) -> Blake2bHash {
        self.genesis_hash
            .expect("Genesis hash should be present. Should be called after run_genesis.")
    }

    pub fn get_post_state_hash(&self) -> Blake2bHash {
        self.post_state_hash.expect("Should have post-state hash.")
    }

    pub fn get_engine_state(&self) -> &EngineState<S> {
        &self.engine_state
    }

    pub fn get_exec_results(&self) -> &Vec<Vec<Rc<ExecutionResult>>> {
        &self.exec_results
    }

    pub fn get_exec_result(&self, index: usize) -> Option<&Vec<Rc<ExecutionResult>>> {
        self.exec_results.get(index)
    }

    pub fn get_exec_results_count(&self) -> usize {
        self.exec_results.len()
    }

    pub fn get_upgrade_result(
        &self,
        index: usize,
    ) -> Option<&Result<UpgradeResult, engine_state::Error>> {
        self.upgrade_results.get(index)
    }

    pub fn expect_upgrade_success(&mut self) -> &mut Self {
        // Check first result, as only first result is interesting for a simple test
        let result = self
            .upgrade_results
            .last()
            .expect("Expected to be called after a system upgrade.")
            .as_ref();

        result.unwrap_or_else(|_| panic!("Expected success, got: {:?}", result));

        self
    }

    pub fn finish(&self) -> WasmTestResult<S> {
        WasmTestResult(self.clone())
    }

    pub fn get_handle_payment_contract(&self) -> Contract {
        let handle_payment_contract: Key = self
            .handle_payment_contract_hash
            .expect("should have handle payment contract uref")
            .into();
        self.query(None, handle_payment_contract, &[])
            .and_then(|v| v.try_into().map_err(|error| format!("{:?}", error)))
            .expect("should find handle payment URef")
    }

    pub fn get_purse_balance(&self, purse: URef) -> U512 {
        let base_key = Key::Balance(purse.addr());
        self.query(None, base_key, &[])
            .and_then(|v| CLValue::try_from(v).map_err(|error| format!("{:?}", error)))
            .and_then(|cl_value| cl_value.into_t().map_err(|error| format!("{:?}", error)))
            .expect("should parse balance into a U512")
    }

    pub fn get_purse_balance_result(&self, purse: URef) -> BalanceResult {
        let correlation_id = CorrelationId::new();
        let state_root_hash: Blake2bHash =
            self.post_state_hash.expect("should have post_state_hash");
        self.engine_state
            .get_purse_balance(correlation_id, state_root_hash, purse)
            .expect("should get purse balance")
    }

    pub fn get_public_key_balance_result(&self, public_key: PublicKey) -> BalanceResult {
        let correlation_id = CorrelationId::new();
        let state_root_hash: Blake2bHash =
            self.post_state_hash.expect("should have post_state_hash");
        self.engine_state
            .get_balance(correlation_id, state_root_hash, public_key)
            .expect("should get purse balance using public key")
    }

    pub fn get_proposer_purse_balance(&self) -> U512 {
        let proposer_account = self
            .get_account(*DEFAULT_PROPOSER_ADDR)
            .expect("proposer account should exist");
        self.get_purse_balance(proposer_account.main_purse())
    }

    pub fn get_account(&self, account_hash: AccountHash) -> Option<Account> {
        match self.query(None, Key::Account(account_hash), &[]) {
            Ok(account_value) => match account_value {
                StoredValue::Account(account) => Some(account),
                _ => None,
            },
            Err(_) => None,
        }
    }

    pub fn get_contract(&self, contract_hash: ContractHash) -> Option<Contract> {
        let contract_value: StoredValue = self
            .query(None, contract_hash.into(), &[])
            .expect("should have contract value");

        if let StoredValue::Contract(contract) = contract_value {
            Some(contract)
        } else {
            None
        }
    }

    pub fn get_contract_wasm(&self, contract_hash: ContractHash) -> Option<ContractWasm> {
        let contract_value: StoredValue = self
            .query(None, contract_hash.into(), &[])
            .expect("should have contract value");

        if let StoredValue::ContractWasm(contract_wasm) = contract_value {
            Some(contract_wasm)
        } else {
            None
        }
    }

    pub fn get_contract_package(
        &self,
        contract_package_hash: ContractPackageHash,
    ) -> Option<ContractPackage> {
        let contract_value: StoredValue = self
            .query(None, contract_package_hash.into(), &[])
            .expect("should have package value");

        if let StoredValue::ContractPackage(package) = contract_value {
            Some(package)
        } else {
            None
        }
    }

    pub fn get_transfer(&self, transfer: TransferAddr) -> Option<Transfer> {
        let transfer_value: StoredValue = self
            .query(None, Key::Transfer(transfer), &[])
            .expect("should have transfer value");

        if let StoredValue::Transfer(transfer) = transfer_value {
            Some(transfer)
        } else {
            None
        }
    }

    pub fn get_deploy_info(&self, deploy_hash: DeployHash) -> Option<DeployInfo> {
        let deploy_info_value: StoredValue = self
            .query(None, Key::DeployInfo(deploy_hash), &[])
            .expect("should have deploy info value");

        if let StoredValue::DeployInfo(deploy_info) = deploy_info_value {
            Some(deploy_info)
        } else {
            None
        }
    }

    pub fn exec_costs(&self, index: usize) -> Vec<Gas> {
        let exec_results = self
            .get_exec_result(index)
            .expect("should have exec response");
        utils::get_exec_costs(exec_results)
    }

    pub fn last_exec_gas_cost(&self) -> Gas {
        let exec_results = self
            .exec_results
            .last()
            .expect("Expected to be called after run()");
        let exec_result = exec_results.get(0).expect("should have result");
        exec_result.cost()
    }

    pub fn exec_error_message(&self, index: usize) -> Option<String> {
        let response = self.get_exec_result(index)?;
        Some(utils::get_error_message(response))
    }

    pub fn exec_commit_finish(&mut self, execute_request: ExecuteRequest) -> WasmTestResult<S> {
        self.exec(execute_request)
            .expect_success()
            .commit()
            .finish()
    }

    pub fn get_era_validators(&mut self) -> EraValidators {
        let correlation_id = CorrelationId::new();
        let state_hash = self.get_post_state_hash();
        let request = GetEraValidatorsRequest::new(state_hash, *DEFAULT_PROTOCOL_VERSION);
        self.engine_state
            .get_era_validators(correlation_id, request)
            .expect("get era validators should not error")
    }

    pub fn get_validator_weights(&mut self, era_id: EraId) -> Option<ValidatorWeights> {
        let mut result = self.get_era_validators();
        result.remove(&era_id)
    }

    pub fn get_bids(&mut self) -> Bids {
        let get_bids_request = GetBidsRequest::new(self.get_post_state_hash());

        let get_bids_result = self
            .engine_state
            .get_bids(CorrelationId::new(), get_bids_request)
            .unwrap();

        get_bids_result.bids().cloned().unwrap()
    }

    pub fn get_withdraws(&mut self) -> UnbondingPurses {
        let correlation_id = CorrelationId::new();
        let state_root_hash = self.get_post_state_hash();

        let tracking_copy = self
            .engine_state
            .tracking_copy(state_root_hash)
            .unwrap()
            .unwrap();

        let reader = tracking_copy.reader();

        let withdraws_keys = reader
            .keys_with_prefix(correlation_id, &[KeyTag::Withdraw as u8])
            .unwrap_or_default();

        let mut ret = BTreeMap::new();

        for key in withdraws_keys.into_iter() {
            let read_result = reader.read(correlation_id, &key);
            if let (
                Key::Withdraw(account_hash),
                Ok(Some(StoredValue::Withdraw(unbonding_purses))),
            ) = (key, read_result)
            {
                ret.insert(account_hash, unbonding_purses);
            }
        }

        ret
    }

    pub fn get_value<T>(&mut self, contract_hash: ContractHash, name: &str) -> T
    where
        T: FromBytes + CLTyped,
    {
        let contract = self
            .get_contract(contract_hash)
            .expect("should have contract");
        let key = contract
            .named_keys()
            .get(name)
            .expect("should have named key");
        let stored_value = self.query(None, *key, &[]).expect("should query");
        let cl_value = stored_value
            .as_cl_value()
            .cloned()
            .expect("should be cl value");
        let result: T = cl_value.into_t().expect("should convert");
        result
    }

    pub fn get_era(&mut self) -> EraId {
        let auction_contract = self.get_auction_contract_hash();
        self.get_value(auction_contract, ERA_ID_KEY)
    }

    pub fn get_auction_delay(&mut self) -> u64 {
        let auction_contract = self.get_auction_contract_hash();
        self.get_value(auction_contract, AUCTION_DELAY_KEY)
    }
}
