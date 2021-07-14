use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    iter::FromIterator,
    rc::Rc,
};

use once_cell::sync::Lazy;

use casper_types::{
    account::{AccountHash, Weight},
    contracts::NamedKeys,
    AccessRights, BlockTime, DeployHash, EntryPointType, Key, Phase, ProtocolVersion, RuntimeArgs,
    URef, U512,
};

use super::{Address, Error, RuntimeContext};
use crate::{
    core::{execution::AddressGenerator, tracking_copy::TrackingCopy},
    shared::{
        account::{Account, AssociatedKeys},
        additive_map::AdditiveMap,
        gas::Gas,
        gas_counter::GasCounter,
        newtypes::CorrelationId,
        stored_value::StoredValue,
        transform::Transform,
    },
    storage::{
        global_state::{
            in_memory::{InMemoryGlobalState, InMemoryGlobalStateView},
            CommitResult, StateProvider,
        },
        protocol_data::ProtocolData,
    },
};

pub const GAS_LIMIT: u64 = u64::MAX;

pub static TEST_PROTOCOL_DATA: Lazy<ProtocolData> = Lazy::new(ProtocolData::default);

pub fn mock_tracking_copy(
    init_key: Key,
    init_account: Account,
) -> TrackingCopy<InMemoryGlobalStateView> {
    let correlation_id = CorrelationId::new();
    let hist = InMemoryGlobalState::empty().unwrap();
    let root_hash = hist.empty_root_hash;
    let transform = Transform::Write(StoredValue::Account(init_account));

    let mut m = AdditiveMap::new();
    m.insert(init_key, transform);
    let commit_result = hist
        .commit(correlation_id, root_hash, m)
        .expect("Creation of mocked account should be a success.");

    let new_hash = match commit_result {
        CommitResult::Success { state_root, .. } => state_root,
        other => panic!("Commiting changes to test History failed: {:?}.", other),
    };

    let reader = hist
        .checkout(new_hash)
        .expect("Checkout should not throw errors.")
        .expect("Root hash should exist.");

    TrackingCopy::new(reader)
}

pub fn mock_account_with_purse(account_hash: AccountHash, purse: [u8; 32]) -> (Key, Account) {
    let associated_keys = AssociatedKeys::new(account_hash, Weight::new(1));
    let account = Account::new(
        account_hash,
        NamedKeys::new(),
        URef::new(purse, AccessRights::READ_ADD_WRITE),
        associated_keys,
        Default::default(),
    );
    let key = Key::Account(account_hash);

    (key, account)
}

pub fn mock_account(account_hash: AccountHash) -> (Key, Account) {
    mock_account_with_purse(account_hash, [0; 32])
}

pub fn mock_runtime_context<'a>(
    account: &'a Account,
    base_key: Key,
    named_keys: &'a mut NamedKeys,
    access_rights: HashMap<Address, HashSet<AccessRights>>,
    hash_address_generator: AddressGenerator,
    uref_address_generator: AddressGenerator,
    transfer_address_generator: AddressGenerator,
) -> RuntimeContext<'a, InMemoryGlobalStateView> {
    let tracking_copy = mock_tracking_copy(base_key, account.clone());
    RuntimeContext::new(
        Rc::new(RefCell::new(tracking_copy)),
        EntryPointType::Session,
        named_keys,
        access_rights,
        RuntimeArgs::new(),
        BTreeSet::from_iter(vec![AccountHash::new([0; 32])]),
        account,
        base_key,
        BlockTime::new(0),
        DeployHash::new([1u8; 32]),
        GasCounter::new(Gas::new(U512::from(GAS_LIMIT)), Gas::default()),
        Rc::new(RefCell::new(hash_address_generator)),
        Rc::new(RefCell::new(uref_address_generator)),
        Rc::new(RefCell::new(transfer_address_generator)),
        ProtocolVersion::V1_0_0,
        CorrelationId::new(),
        Phase::Session,
        *TEST_PROTOCOL_DATA,
        Vec::default(),
    )
}

pub fn test<T, F>(
    access_rights: HashMap<Address, HashSet<AccessRights>>,
    query: F,
) -> Result<T, Error>
where
    F: FnOnce(RuntimeContext<InMemoryGlobalStateView>) -> Result<T, Error>,
{
    let deploy_hash = [1u8; 32];
    let (base_key, account) = mock_account(AccountHash::new([0u8; 32]));

    let mut named_keys = NamedKeys::new();
    let uref_address_generator = AddressGenerator::new(&deploy_hash, Phase::Session);
    let hash_address_generator = AddressGenerator::new(&deploy_hash, Phase::Session);
    let transfer_address_generator = AddressGenerator::new(&deploy_hash, Phase::Session);
    let runtime_context = mock_runtime_context(
        &account,
        base_key,
        &mut named_keys,
        access_rights,
        hash_address_generator,
        uref_address_generator,
        transfer_address_generator,
    );
    query(runtime_context)
}
