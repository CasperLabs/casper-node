use std::collections::{BTreeMap, BTreeSet};

use once_cell::sync::Lazy;

use casper_engine_test_support::{
    internal::{ExecuteRequestBuilder, InMemoryWasmTestBuilder, DEFAULT_RUN_GENESIS_REQUEST},
    DEFAULT_ACCOUNT_ADDR,
};
use casper_execution_engine::storage::protocol_data::DEFAULT_WASMLESS_TRANSFER_COST;
use casper_types::{
    account::AccountHash, runtime_args, AccessRights, DeployHash, PublicKey, RuntimeArgs,
    SecretKey, Transfer, TransferAddr, U512,
};

const CONTRACT_TRANSFER_PURSE_TO_ACCOUNT: &str = "transfer_purse_to_account.wasm";
const CONTRACT_TRANSFER_PURSE_TO_ACCOUNT_WITH_ID: &str = "transfer_purse_to_account_with_id.wasm";
const TRANSFER_ARG_TARGET: &str = "target";
const TRANSFER_ARG_AMOUNT: &str = "amount";
const TRANSFER_ARG_ID: &str = "id";

const CONTRACT_TRANSFER_PURSE_TO_ACCOUNTS: &str = "transfer_purse_to_accounts.wasm";
const TRANSFER_ARG_SOURCE: &str = "source";
const TRANSFER_ARG_TARGETS: &str = "targets";

const CONTRACT_TRANSFER_PURSE_TO_ACCOUNTS_STORED: &str = "transfer_purse_to_accounts_stored.wasm";
const CONTRACT_TRANSFER_PURSE_TO_ACCOUNTS_SUBCALL: &str = "transfer_purse_to_accounts_subcall.wasm";

static ALICE_KEY: Lazy<PublicKey> = Lazy::new(|| {
    let secret_key = SecretKey::ed25519_from_bytes([3; 32]).unwrap();
    PublicKey::from(&secret_key)
});
static BOB_KEY: Lazy<PublicKey> = Lazy::new(|| {
    let secret_key = SecretKey::ed25519_from_bytes([5; 32]).unwrap();
    PublicKey::from(&secret_key)
});
static CAROL_KEY: Lazy<PublicKey> = Lazy::new(|| {
    let secret_key = SecretKey::ed25519_from_bytes([7; 32]).unwrap();
    PublicKey::from(&secret_key)
});

static ALICE_ADDR: Lazy<AccountHash> = Lazy::new(|| AccountHash::from(&*ALICE_KEY));
static BOB_ADDR: Lazy<AccountHash> = Lazy::new(|| AccountHash::from(&*BOB_KEY));
static CAROL_ADDR: Lazy<AccountHash> = Lazy::new(|| AccountHash::from(&*CAROL_KEY));

static TRANSFER_AMOUNT_1: Lazy<U512> = Lazy::new(|| U512::from(100_100_000));
static TRANSFER_AMOUNT_2: Lazy<U512> = Lazy::new(|| U512::from(200_100_000));
static TRANSFER_AMOUNT_3: Lazy<U512> = Lazy::new(|| U512::from(300_100_000));

#[ignore]
#[test]
fn should_record_wasmless_transfer() {
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let id = Some(0);

    let transfer_request = ExecuteRequestBuilder::transfer(
        *DEFAULT_ACCOUNT_ADDR,
        runtime_args! {
            TRANSFER_ARG_TARGET => *ALICE_ADDR,
            TRANSFER_ARG_AMOUNT => *TRANSFER_AMOUNT_1,
            TRANSFER_ARG_ID => id
        },
    )
    .build();

    let deploy_hash = {
        let deploy_items: Vec<DeployHash> = transfer_request
            .deploys()
            .iter()
            .map(|deploy_item| deploy_item.deploy_hash)
            .collect();
        deploy_items[0]
    };

    builder.exec(transfer_request).commit().expect_success();

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have default account");

    let alice_account = builder
        .get_account(*ALICE_ADDR)
        .expect("should have Alice's account");

    let alice_attenuated_main_purse = alice_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let deploy_info = builder
        .get_deploy_info(deploy_hash)
        .expect("should have deploy info");

    assert_eq!(deploy_info.deploy_hash, deploy_hash);
    assert_eq!(deploy_info.from, *DEFAULT_ACCOUNT_ADDR);
    assert_eq!(deploy_info.source, default_account.main_purse());

    assert_eq!(deploy_info.gas, U512::from(DEFAULT_WASMLESS_TRANSFER_COST));

    let transfers = deploy_info.transfers;
    assert_eq!(transfers.len(), 1);

    let transfer = builder
        .get_transfer(transfers[0])
        .expect("should have transfer");

    assert_eq!(transfer.deploy_hash(), &deploy_hash);
    assert_eq!(transfer.from(), &*DEFAULT_ACCOUNT_ADDR);
    assert_eq!(transfer.to(), &Some(*ALICE_ADDR));
    assert_eq!(transfer.source(), &default_account.main_purse());
    assert_eq!(transfer.target(), &alice_attenuated_main_purse);
    assert_eq!(transfer.amount(), &*TRANSFER_AMOUNT_1);
    assert_eq!(transfer.id(), &id);
}

#[ignore]
#[test]
fn should_record_wasm_transfer() {
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let transfer_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNT,
        runtime_args! {
            TRANSFER_ARG_TARGET => *ALICE_ADDR,
            TRANSFER_ARG_AMOUNT => *TRANSFER_AMOUNT_1
        },
    )
    .build();

    let deploy_hash = {
        let deploy_items: Vec<DeployHash> = transfer_request
            .deploys()
            .iter()
            .map(|deploy_item| deploy_item.deploy_hash)
            .collect();
        deploy_items[0]
    };

    builder.exec(transfer_request).commit().expect_success();

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have default account");

    let alice_account = builder
        .get_account(*ALICE_ADDR)
        .expect("should have Alice's account");

    let alice_attenuated_main_purse = alice_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let deploy_info = builder
        .get_deploy_info(deploy_hash)
        .expect("should have deploy info");

    assert_eq!(deploy_info.deploy_hash, deploy_hash);
    assert_eq!(deploy_info.from, *DEFAULT_ACCOUNT_ADDR);
    assert_eq!(deploy_info.source, default_account.main_purse());
    assert_ne!(deploy_info.gas, U512::zero());

    let transfers = deploy_info.transfers;
    assert_eq!(transfers.len(), 1);

    let transfer = builder
        .get_transfer(transfers[0])
        .expect("should have transfer");

    assert_eq!(transfer.deploy_hash(), &deploy_hash);
    assert_eq!(transfer.from(), &*DEFAULT_ACCOUNT_ADDR);
    assert_eq!(transfer.source(), &default_account.main_purse());
    assert_eq!(transfer.target(), &alice_attenuated_main_purse);
    assert_eq!(transfer.amount(), &*TRANSFER_AMOUNT_1);
}

#[ignore]
#[test]
fn should_record_wasm_transfer_with_id() {
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let id = Some(0);

    let transfer_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNT_WITH_ID,
        runtime_args! {
            TRANSFER_ARG_TARGET => *ALICE_ADDR,
            TRANSFER_ARG_AMOUNT => *TRANSFER_AMOUNT_1,
            TRANSFER_ARG_ID => id
        },
    )
    .build();

    let deploy_hash = {
        let deploy_items: Vec<DeployHash> = transfer_request
            .deploys()
            .iter()
            .map(|deploy_item| deploy_item.deploy_hash)
            .collect();
        deploy_items[0]
    };

    builder.exec(transfer_request).commit().expect_success();

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have default account");

    let alice_account = builder
        .get_account(*ALICE_ADDR)
        .expect("should have Alice's account");

    let alice_attenuated_main_purse = alice_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let deploy_info = builder
        .get_deploy_info(deploy_hash)
        .expect("should have deploy info");

    assert_eq!(deploy_info.deploy_hash, deploy_hash);
    assert_eq!(deploy_info.from, *DEFAULT_ACCOUNT_ADDR);
    assert_eq!(deploy_info.source, default_account.main_purse());
    assert_ne!(deploy_info.gas, U512::zero());

    let transfers = deploy_info.transfers;
    assert_eq!(transfers.len(), 1);

    let transfer = builder
        .get_transfer(transfers[0])
        .expect("should have transfer");

    assert_eq!(transfer.deploy_hash(), &deploy_hash);
    assert_eq!(transfer.from(), &*DEFAULT_ACCOUNT_ADDR);
    assert_eq!(transfer.source(), &default_account.main_purse());
    assert_eq!(transfer.target(), &alice_attenuated_main_purse);
    assert_eq!(transfer.amount(), &*TRANSFER_AMOUNT_1);
    assert_eq!(transfer.id(), &id);
}

#[ignore]
#[test]
fn should_record_wasm_transfers() {
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have default account");

    let alice_id = Some(0);
    let bob_id = Some(1);
    let carol_id = Some(2);

    let targets: BTreeMap<AccountHash, (U512, Option<u64>)> = {
        let mut tmp = BTreeMap::new();
        tmp.insert(*ALICE_ADDR, (*TRANSFER_AMOUNT_1, alice_id));
        tmp.insert(*BOB_ADDR, (*TRANSFER_AMOUNT_2, bob_id));
        tmp.insert(*CAROL_ADDR, (*TRANSFER_AMOUNT_3, carol_id));
        tmp
    };

    let transfer_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNTS,
        runtime_args! {
            TRANSFER_ARG_SOURCE => default_account.main_purse(),
            TRANSFER_ARG_TARGETS => targets,
        },
    )
    .build();

    let deploy_hash = {
        let deploy_items: Vec<DeployHash> = transfer_request
            .deploys()
            .iter()
            .map(|deploy_item| deploy_item.deploy_hash)
            .collect();
        deploy_items[0]
    };

    builder.exec(transfer_request).commit().expect_success();

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have default account");

    let alice_account = builder
        .get_account(*ALICE_ADDR)
        .expect("should have Alice's account");

    let bob_account = builder
        .get_account(*BOB_ADDR)
        .expect("should have Bob's account");

    let carol_account = builder
        .get_account(*CAROL_ADDR)
        .expect("should have Carol's account");

    let alice_attenuated_main_purse = alice_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let bob_attenuated_main_purse = bob_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let carol_attenuated_main_purse = carol_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let deploy_info = builder
        .get_deploy_info(deploy_hash)
        .expect("should have deploy info");

    assert_eq!(deploy_info.deploy_hash, deploy_hash);
    assert_eq!(deploy_info.from, *DEFAULT_ACCOUNT_ADDR);
    assert_eq!(deploy_info.source, default_account.main_purse());
    assert_ne!(deploy_info.gas, U512::zero());

    const EXPECTED_LENGTH: usize = 3;
    let transfer_addrs = deploy_info.transfers;
    assert_eq!(transfer_addrs.len(), EXPECTED_LENGTH);
    assert_eq!(
        transfer_addrs
            .iter()
            .cloned()
            .collect::<BTreeSet<TransferAddr>>()
            .len(),
        EXPECTED_LENGTH
    );

    let transfers: BTreeSet<Transfer> = {
        let mut tmp = BTreeSet::new();
        for transfer_addr in transfer_addrs {
            let transfer = builder
                .get_transfer(transfer_addr)
                .expect("should have transfer");
            tmp.insert(transfer);
        }
        tmp
    };

    assert_eq!(transfers.len(), EXPECTED_LENGTH);

    assert!(transfers.contains(&Transfer::new(
        deploy_hash,
        *DEFAULT_ACCOUNT_ADDR,
        Some(*ALICE_ADDR),
        default_account.main_purse(),
        alice_attenuated_main_purse,
        *TRANSFER_AMOUNT_1,
        alice_id,
    )));

    assert!(transfers.contains(&Transfer::new(
        deploy_hash,
        *DEFAULT_ACCOUNT_ADDR,
        Some(*BOB_ADDR),
        default_account.main_purse(),
        bob_attenuated_main_purse,
        *TRANSFER_AMOUNT_2,
        bob_id,
    )));

    assert!(transfers.contains(&Transfer::new(
        deploy_hash,
        *DEFAULT_ACCOUNT_ADDR,
        Some(*CAROL_ADDR),
        default_account.main_purse(),
        carol_attenuated_main_purse,
        *TRANSFER_AMOUNT_3,
        carol_id,
    )));
}

#[ignore]
#[test]
fn should_record_wasm_transfers_with_subcall() {
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let alice_id = Some(0);
    let bob_id = Some(1);
    let carol_id = Some(2);

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have default account");

    let store_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNTS_STORED,
        runtime_args! {},
    )
    .build();

    let targets: BTreeMap<AccountHash, (U512, Option<u64>)> = {
        let mut tmp = BTreeMap::new();
        tmp.insert(*ALICE_ADDR, (*TRANSFER_AMOUNT_1, alice_id));
        tmp.insert(*BOB_ADDR, (*TRANSFER_AMOUNT_2, bob_id));
        tmp.insert(*CAROL_ADDR, (*TRANSFER_AMOUNT_3, carol_id));
        tmp
    };

    let transfer_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNTS_SUBCALL,
        runtime_args! {
            TRANSFER_ARG_SOURCE => default_account.main_purse(),
            TRANSFER_ARG_TARGETS => targets,
        },
    )
    .build();

    let transfer_deploy_hash = {
        let deploy_items: Vec<DeployHash> = transfer_request
            .deploys()
            .iter()
            .map(|deploy_item| deploy_item.deploy_hash)
            .collect();
        deploy_items[0]
    };

    builder.exec(store_request).commit().expect_success();
    builder.exec(transfer_request).commit().expect_success();

    let alice_account = builder
        .get_account(*ALICE_ADDR)
        .expect("should have Alice's account");

    let bob_account = builder
        .get_account(*BOB_ADDR)
        .expect("should have Bob's account");

    let carol_account = builder
        .get_account(*CAROL_ADDR)
        .expect("should have Carol's account");

    let alice_attenuated_main_purse = alice_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let bob_attenuated_main_purse = bob_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let carol_attenuated_main_purse = carol_account
        .main_purse()
        .with_access_rights(AccessRights::ADD);

    let deploy_info = builder
        .get_deploy_info(transfer_deploy_hash)
        .expect("should have deploy info");

    assert_eq!(deploy_info.deploy_hash, transfer_deploy_hash);
    assert_eq!(deploy_info.from, *DEFAULT_ACCOUNT_ADDR);
    assert_eq!(deploy_info.source, default_account.main_purse());
    assert_ne!(deploy_info.gas, U512::zero());

    const EXPECTED_LENGTH: usize = 6;
    let transfer_addrs = deploy_info.transfers;
    assert_eq!(transfer_addrs.len(), EXPECTED_LENGTH);
    assert_eq!(
        transfer_addrs
            .iter()
            .cloned()
            .collect::<BTreeSet<TransferAddr>>()
            .len(),
        EXPECTED_LENGTH
    );

    let transfer_counts: BTreeMap<Transfer, usize> = {
        let mut tmp = BTreeMap::new();
        for transfer_addr in transfer_addrs {
            let transfer = builder
                .get_transfer(transfer_addr)
                .expect("should have transfer");
            tmp.entry(transfer).and_modify(|i| *i += 1).or_insert(1);
        }
        tmp
    };

    let expected_alice = Transfer::new(
        transfer_deploy_hash,
        *DEFAULT_ACCOUNT_ADDR,
        Some(*ALICE_ADDR),
        default_account.main_purse(),
        alice_attenuated_main_purse,
        *TRANSFER_AMOUNT_1,
        alice_id,
    );

    let expected_bob = Transfer::new(
        transfer_deploy_hash,
        *DEFAULT_ACCOUNT_ADDR,
        Some(*BOB_ADDR),
        default_account.main_purse(),
        bob_attenuated_main_purse,
        *TRANSFER_AMOUNT_2,
        bob_id,
    );

    let expected_carol = Transfer::new(
        transfer_deploy_hash,
        *DEFAULT_ACCOUNT_ADDR,
        Some(*CAROL_ADDR),
        default_account.main_purse(),
        carol_attenuated_main_purse,
        *TRANSFER_AMOUNT_3,
        carol_id,
    );

    const EXPECTED_COUNT: Option<usize> = Some(2);
    for expected in &[expected_alice, expected_bob, expected_carol] {
        assert_eq!(transfer_counts.get(&expected).cloned(), EXPECTED_COUNT);
    }
}
