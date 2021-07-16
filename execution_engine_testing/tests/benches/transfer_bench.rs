use std::{path::Path, time::Duration};

use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion, Throughput,
};
use tempfile::TempDir;

use casper_engine_test_support::{
    internal::{
        DeployItemBuilder, ExecuteRequestBuilder, LmdbWasmTestBuilder, DEFAULT_ACCOUNT_ADDR,
        DEFAULT_PAYMENT, DEFAULT_RUN_GENESIS_REQUEST,
    },
    MINIMUM_ACCOUNT_CREATION_BALANCE,
};
use casper_execution_engine::core::engine_state::{EngineConfig, ExecuteRequest};
use casper_types::{account::AccountHash, runtime_args, Key, RuntimeArgs, URef, U512};

const CONTRACT_CREATE_ACCOUNTS: &str = "create_accounts.wasm";
const CONTRACT_CREATE_PURSES: &str = "create_purses.wasm";
const CONTRACT_TRANSFER_TO_EXISTING_ACCOUNT: &str = "transfer_to_existing_account.wasm";
const CONTRACT_TRANSFER_TO_PURSE: &str = "transfer_to_purse.wasm";

/// Size of batch used in multiple execs benchmark, and multiple deploys per exec cases.
const TRANSFER_BATCH_SIZE: u64 = 3;
const TARGET_ADDR: AccountHash = AccountHash::new([127; 32]);
const ARG_AMOUNT: &str = "amount";
const ARG_ID: &str = "id";
const ARG_ACCOUNTS: &str = "accounts";
const ARG_SEED_AMOUNT: &str = "seed_amount";
const ARG_TOTAL_PURSES: &str = "total_purses";
const ARG_TARGET: &str = "target";
const ARG_TARGET_PURSE: &str = "target_purse";

const BLOCK_TRANSFER_COUNT: usize = 2500;

/// Converts an integer into an array of type [u8; 32] by converting integer
/// into its big endian representation and embedding it at the end of the
/// range.
fn make_deploy_hash(i: u64) -> [u8; 32] {
    let mut result = [128; 32];
    result[32 - 8..].copy_from_slice(&i.to_be_bytes());
    result
}

fn bootstrap(data_dir: &Path, accounts: Vec<AccountHash>, amount: U512) -> LmdbWasmTestBuilder {
    let exec_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_CREATE_ACCOUNTS,
        runtime_args! { ARG_ACCOUNTS => accounts, ARG_SEED_AMOUNT => amount },
    )
    .build();

    let engine_config = EngineConfig::default();

    let mut builder = LmdbWasmTestBuilder::new_with_config(data_dir, engine_config);

    builder
        .run_genesis(&DEFAULT_RUN_GENESIS_REQUEST)
        .exec(exec_request)
        .expect_success()
        .commit();

    builder
}

fn create_purses(
    builder: &mut LmdbWasmTestBuilder,
    source: AccountHash,
    total_purses: u64,
    purse_amount: U512,
) -> Vec<URef> {
    let exec_request = ExecuteRequestBuilder::standard(
        source,
        CONTRACT_CREATE_PURSES,
        runtime_args! { ARG_TOTAL_PURSES => total_purses, ARG_SEED_AMOUNT => purse_amount },
    )
    .build();

    builder.exec(exec_request).expect_success().commit();

    // Return creates purses for given account by filtering named keys
    let query_result = builder
        .query(None, Key::Account(source), &[])
        .expect("should query target");
    let account = query_result
        .as_account()
        .unwrap_or_else(|| panic!("result should be account but received {:?}", query_result));

    (0..total_purses)
        .map(|index| {
            let purse_lookup_key = format!("purse:{}", index);
            let purse_uref = account
                .named_keys()
                .get(&purse_lookup_key)
                .and_then(Key::as_uref)
                .unwrap_or_else(|| panic!("should get named key {} as uref", purse_lookup_key));
            *purse_uref
        })
        .collect()
}

/// Uses multiple exec requests with a single deploy to transfer tokens. Executes all transfers in
/// batch determined by value of TRANSFER_BATCH_SIZE.
fn transfer_to_account_multiple_execs(
    builder: &mut LmdbWasmTestBuilder,
    account: AccountHash,
    should_commit: bool,
) {
    let amount = U512::one();

    for _ in 0..TRANSFER_BATCH_SIZE {
        let exec_request = ExecuteRequestBuilder::standard(
            *DEFAULT_ACCOUNT_ADDR,
            CONTRACT_TRANSFER_TO_EXISTING_ACCOUNT,
            runtime_args! {
                ARG_TARGET => account,
                ARG_AMOUNT => amount,
            },
        )
        .build();

        let builder = builder.exec(exec_request).expect_success();
        if should_commit {
            builder.commit();
        }
    }
}

/// Executes multiple deploys per single exec with based on TRANSFER_BATCH_SIZE.
fn transfer_to_account_multiple_deploys(
    builder: &mut LmdbWasmTestBuilder,
    account: AccountHash,
    should_commit: bool,
) {
    let mut exec_builder = ExecuteRequestBuilder::new();

    for i in 0..TRANSFER_BATCH_SIZE {
        let deploy = DeployItemBuilder::default()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_empty_payment_bytes(runtime_args! { ARG_AMOUNT => *DEFAULT_PAYMENT })
            .with_session_code(
                CONTRACT_TRANSFER_TO_EXISTING_ACCOUNT,
                runtime_args! {
                    ARG_TARGET => account,
                    ARG_AMOUNT => U512::one(),
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_ADDR])
            .with_deploy_hash(make_deploy_hash(i)) // deploy_hash
            .build();
        exec_builder = exec_builder.push_deploy(deploy);
    }

    let exec_request = exec_builder.build();

    let builder = builder.exec(exec_request).expect_success();
    if should_commit {
        builder.commit();
    }
}

/// Uses multiple exec requests with a single deploy to transfer tokens from purse to purse.
/// Executes all transfers in batch determined by value of TRANSFER_BATCH_SIZE.
fn transfer_to_purse_multiple_execs(
    builder: &mut LmdbWasmTestBuilder,
    purse: URef,
    should_commit: bool,
) {
    let amount = U512::one();

    for _ in 0..TRANSFER_BATCH_SIZE {
        let exec_request = ExecuteRequestBuilder::standard(
            TARGET_ADDR,
            CONTRACT_TRANSFER_TO_PURSE,
            runtime_args! { ARG_TARGET_PURSE => purse, ARG_AMOUNT => amount },
        )
        .build();

        let builder = builder.exec(exec_request).expect_success();
        if should_commit {
            builder.commit();
        }
    }
}

/// Executes multiple deploys per single exec with based on TRANSFER_BATCH_SIZE.
fn transfer_to_purse_multiple_deploys(
    builder: &mut LmdbWasmTestBuilder,
    purse: URef,
    should_commit: bool,
) {
    let mut exec_builder = ExecuteRequestBuilder::new();

    for i in 0..TRANSFER_BATCH_SIZE {
        let deploy = DeployItemBuilder::default()
            .with_address(TARGET_ADDR)
            .with_empty_payment_bytes(runtime_args! { ARG_AMOUNT => *DEFAULT_PAYMENT, })
            .with_session_code(
                CONTRACT_TRANSFER_TO_PURSE,
                runtime_args! { ARG_TARGET_PURSE => purse, ARG_AMOUNT => U512::one() },
            )
            .with_authorization_keys(&[TARGET_ADDR])
            .with_deploy_hash(make_deploy_hash(i)) // deploy_hash
            .build();
        exec_builder = exec_builder.push_deploy(deploy);
    }

    let exec_request = exec_builder.build();

    let builder = builder.exec(exec_request).expect_success();
    if should_commit {
        builder.commit();
    }
}

pub fn transfer_to_existing_accounts(group: &mut BenchmarkGroup<WallTime>, should_commit: bool) {
    let target_account = TARGET_ADDR;
    let bootstrap_accounts = vec![target_account];

    let data_dir = TempDir::new().expect("should create temp dir");
    let mut builder = bootstrap(data_dir.path(), bootstrap_accounts.clone(), U512::one());

    group.bench_function(
        format!(
            "transfer_to_existing_account_multiple_execs/{}/{}",
            TRANSFER_BATCH_SIZE, should_commit
        ),
        |b| {
            b.iter(|| {
                // Execute multiple deploys with multiple exec requests
                transfer_to_account_multiple_execs(&mut builder, target_account, should_commit)
            })
        },
    );

    let data_dir = TempDir::new().expect("should create temp dir");
    let mut builder = bootstrap(data_dir.path(), bootstrap_accounts, U512::one());

    group.bench_function(
        format!(
            "transfer_to_existing_account_multiple_deploys_per_exec/{}/{}",
            TRANSFER_BATCH_SIZE, should_commit
        ),
        |b| {
            b.iter(|| {
                // Execute multiple deploys with a single exec request
                transfer_to_account_multiple_deploys(&mut builder, target_account, should_commit)
            })
        },
    );
}

pub fn multiple_native_transfers(group: &mut BenchmarkGroup<WallTime>) {
    let target_account = TARGET_ADDR;
    let bootstrap_accounts = vec![target_account];

    let data_dir = TempDir::new().expect("should create temp dir");
    let mut builder = bootstrap(
        data_dir.path(),
        bootstrap_accounts,
        U512::from(MINIMUM_ACCOUNT_CREATION_BALANCE),
    );

    let purse_amount = U512::one();
    let purses = create_purses(&mut builder, target_account, 100, purse_amount);

    let mut purse_index = 0usize;
    let mut exec_requests = Vec::with_capacity(BLOCK_TRANSFER_COUNT);
    for _ in 0..BLOCK_TRANSFER_COUNT {
        let account = {
            let account = purses[purse_index];
            if purse_index == purses.len() - 1 {
                purse_index = 0;
            } else {
                purse_index += 1;
            }
            account
        };
        let mut exec_builder = ExecuteRequestBuilder::new();
        let runtime_args = runtime_args! {
            ARG_TARGET => account,
            ARG_AMOUNT => U512::one(),
            ARG_ID => <Option<u64>>::None
        };
        let native_transfer = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_ADDR])
            .build();
        exec_builder = exec_builder.push_deploy(native_transfer);
        let exec_request = exec_builder.build();
        exec_requests.push(exec_request);
    }

    group.bench_function(
        format!("multiple_native_transfers/{}", BLOCK_TRANSFER_COUNT),
        |b| b.iter(|| transfer_to_account_multiple_native_transfers(&mut builder, &exec_requests)),
    );
}

fn transfer_to_account_multiple_native_transfers(
    builder: &mut LmdbWasmTestBuilder,
    execute_requests: &[ExecuteRequest],
) {
    for exec_request in execute_requests.iter().cloned() {
        let builder = builder.exec(exec_request).expect_success();
        builder.commit();
    }
    // flush to disk only after entire set of deploys has been executed
    builder.flush_environment();
}

pub fn transfer_to_existing_purses(group: &mut BenchmarkGroup<WallTime>, should_commit: bool) {
    let target_account = TARGET_ADDR;
    let bootstrap_accounts = vec![target_account];

    let data_dir = TempDir::new().expect("should create temp dir");
    let mut builder = bootstrap(
        data_dir.path(),
        bootstrap_accounts.clone(),
        U512::from(MINIMUM_ACCOUNT_CREATION_BALANCE),
    );

    let purse_amount = U512::one();
    let purses = create_purses(&mut builder, target_account, 1, purse_amount);

    group.bench_function(
        format!(
            "transfer_to_purse_multiple_execs/{}/{}",
            TRANSFER_BATCH_SIZE, should_commit
        ),
        |b| {
            let target_purse = purses[0];
            b.iter(|| {
                // Execute multiple deploys with multiple exec request
                transfer_to_purse_multiple_execs(&mut builder, target_purse, should_commit)
            })
        },
    );

    let data_dir = TempDir::new().expect("should create temp dir");
    let mut builder = bootstrap(
        data_dir.path(),
        bootstrap_accounts,
        U512::from(MINIMUM_ACCOUNT_CREATION_BALANCE) * 10,
    );
    let purses = create_purses(&mut builder, TARGET_ADDR, 1, U512::one());

    group.bench_function(
        format!(
            "transfer_to_purse_multiple_deploys_per_exec/{}/{}",
            TRANSFER_BATCH_SIZE, should_commit
        ),
        |b| {
            let target_purse = purses[0];
            b.iter(|| {
                // Execute multiple deploys with a single exec request
                transfer_to_purse_multiple_deploys(&mut builder, target_purse, should_commit)
            })
        },
    );
}

pub fn native_transfer_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("tps_native");

    // Minimum number of samples and measurement times to decrease the total time of this benchmark.
    // This may or may not decrease the quality of the numbers.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    // Measure by elements where one element per second is one transaction per second
    group.throughput(Throughput::Elements(BLOCK_TRANSFER_COUNT as u64));

    multiple_native_transfers(&mut group);

    group.finish();
}

pub fn transfer_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("tps");

    // Minimum number of samples and measurement times to decrease the total time of this benchmark.
    // This may or may not decrease the quality of the numbers.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    // Measure by elements where one element per second is one transaction per second
    group.throughput(Throughput::Elements(TRANSFER_BATCH_SIZE));

    // Transfers to existing accounts, no commits
    transfer_to_existing_accounts(&mut group, false);

    // Transfers to existing purses, no commits
    transfer_to_existing_purses(&mut group, false);

    // Transfers to existing accounts, with commits
    transfer_to_existing_accounts(&mut group, true);

    // Transfers to existing purses, with commits
    transfer_to_existing_purses(&mut group, true);

    group.finish();
}

criterion_group!(transfer_benches, native_transfer_bench);
criterion_group!(benches, transfer_bench);
criterion_main!(benches, transfer_benches);
