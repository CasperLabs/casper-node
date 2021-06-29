use std::collections::BTreeMap;

use casper_engine_test_support::{
    internal::{
        ExecuteRequestBuilder, InMemoryWasmTestBuilder, UpgradeRequestBuilder,
        DEFAULT_MAX_ASSOCIATED_KEYS, DEFAULT_RUN_GENESIS_REQUEST, DEFAULT_UNBONDING_DELAY,
        DEFAULT_WASM_CONFIG,
    },
    AccountHash, DEFAULT_ACCOUNT_ADDR,
};

use casper_execution_engine::shared::{
    core_config::CoreConfig,
    host_function_costs::HostFunctionCosts,
    opcode_costs::{
        OpcodeCosts, DEFAULT_ADD_COST, DEFAULT_BIT_COST, DEFAULT_CONST_COST,
        DEFAULT_CONTROL_FLOW_COST, DEFAULT_CONVERSION_COST, DEFAULT_CURRENT_MEMORY_COST,
        DEFAULT_DIV_COST, DEFAULT_GLOBAL_COST, DEFAULT_GROW_MEMORY_COST,
        DEFAULT_INTEGER_COMPARISON_COST, DEFAULT_LOAD_COST, DEFAULT_LOCAL_COST, DEFAULT_MUL_COST,
        DEFAULT_NOP_COST, DEFAULT_REGULAR_COST, DEFAULT_STORE_COST, DEFAULT_UNREACHABLE_COST,
    },
    storage_costs::StorageCosts,
    stored_value::StoredValue,
    wasm_config::{WasmConfig, DEFAULT_MAX_STACK_HEIGHT, DEFAULT_WASM_MAX_MEMORY},
};
use casper_types::{
    account::AccountHashBytes,
    runtime_args,
    system::{
        auction::{
            AUCTION_DELAY_KEY, LOCKED_FUNDS_PERIOD_KEY, UNBONDING_DELAY_KEY, VALIDATOR_SLOTS_KEY,
        },
        mint::ROUND_SEIGNIORAGE_RATE_KEY,
    },
    CLValue, EraId, ProtocolVersion, RuntimeArgs, U256, U512,
};
use num_rational::Ratio;

const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V1_0_0;
const DEFAULT_ACTIVATION_POINT: EraId = EraId::new(1);
const ARG_ACCOUNT: &str = "account";

fn get_upgraded_wasm_config() -> WasmConfig {
    let opcode_cost = OpcodeCosts {
        bit: DEFAULT_BIT_COST + 1,
        add: DEFAULT_ADD_COST + 1,
        mul: DEFAULT_MUL_COST + 1,
        div: DEFAULT_DIV_COST + 1,
        load: DEFAULT_LOAD_COST + 1,
        store: DEFAULT_STORE_COST + 1,
        op_const: DEFAULT_CONST_COST + 1,
        local: DEFAULT_LOCAL_COST + 1,
        global: DEFAULT_GLOBAL_COST + 1,
        control_flow: DEFAULT_CONTROL_FLOW_COST + 1,
        integer_comparison: DEFAULT_INTEGER_COMPARISON_COST + 1,
        conversion: DEFAULT_CONVERSION_COST + 1,
        unreachable: DEFAULT_UNREACHABLE_COST + 1,
        nop: DEFAULT_NOP_COST + 1,
        current_memory: DEFAULT_CURRENT_MEMORY_COST + 1,
        grow_memory: DEFAULT_GROW_MEMORY_COST + 1,
        regular: DEFAULT_REGULAR_COST + 1,
    };
    let storage_costs = StorageCosts::default();
    let host_function_costs = HostFunctionCosts::default();
    WasmConfig::new(
        DEFAULT_WASM_MAX_MEMORY,
        DEFAULT_MAX_STACK_HEIGHT * 2,
        opcode_cost,
        storage_costs,
        host_function_costs,
    )
}

#[ignore]
#[test]
fn should_upgrade_only_protocol_version() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let upgraded_protocol_data = builder
        .get_engine_state()
        .get_protocol_data(new_protocol_version)
        .expect("should have result")
        .expect("should have protocol data");

    assert_eq!(
        *DEFAULT_WASM_CONFIG,
        *upgraded_protocol_data.wasm_config(),
        "upgraded costs should equal original costs"
    );
}

#[ignore]
#[test]
fn should_allow_only_wasm_costs_patch_version() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 2);

    let new_wasm_config = get_upgraded_wasm_config();

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_wasm_config(new_wasm_config)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let upgraded_protocol_data = builder
        .get_engine_state()
        .get_protocol_data(new_protocol_version)
        .expect("should have result")
        .expect("should have upgraded protocol data");

    assert_eq!(
        new_wasm_config,
        *upgraded_protocol_data.wasm_config(),
        "upgraded costs should equal new costs"
    );
}

#[ignore]
#[test]
fn should_allow_only_wasm_costs_minor_version() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor + 1, sem_ver.patch);

    let new_wasm_config = get_upgraded_wasm_config();

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_wasm_config(new_wasm_config)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let upgraded_protocol_data = builder
        .get_engine_state()
        .get_protocol_data(new_protocol_version)
        .expect("should have result")
        .expect("should have upgraded protocol data");

    assert_eq!(
        new_wasm_config,
        *upgraded_protocol_data.wasm_config(),
        "upgraded costs should equal new costs"
    );
}

#[ignore]
#[test]
fn should_not_downgrade() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let new_protocol_version = ProtocolVersion::from_parts(2, 0, 0);

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let upgraded_protocol_data = builder
        .get_engine_state()
        .get_protocol_data(new_protocol_version)
        .expect("should have result")
        .expect("should have protocol data");

    assert_eq!(
        *DEFAULT_WASM_CONFIG,
        *upgraded_protocol_data.wasm_config(),
        "upgraded costs should equal original costs"
    );

    let mut downgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(new_protocol_version)
            .with_new_protocol_version(PROTOCOL_VERSION)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder.upgrade_with_upgrade_request(&mut downgrade_request);

    let maybe_upgrade_result = builder.get_upgrade_result(1).expect("should have response");

    assert!(
        maybe_upgrade_result.is_err(),
        "expected failure got {:?}",
        maybe_upgrade_result
    );
}

#[ignore]
#[test]
fn should_not_skip_major_versions() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();

    let invalid_version =
        ProtocolVersion::from_parts(sem_ver.major + 2, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(invalid_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder.upgrade_with_upgrade_request(&mut upgrade_request);

    let maybe_upgrade_result = builder.get_upgrade_result(0).expect("should have response");

    assert!(maybe_upgrade_result.is_err(), "expected failure");
}

#[ignore]
#[test]
fn should_allow_skip_minor_versions() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();

    // can skip minor versions as long as they are higher than current version
    let valid_new_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor + 2, sem_ver.patch);

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(valid_new_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder.upgrade_with_upgrade_request(&mut upgrade_request);

    let maybe_upgrade_result = builder.get_upgrade_result(0).expect("should have response");

    assert!(!maybe_upgrade_result.is_err(), "expected success");
}

#[ignore]
#[test]
fn should_upgrade_only_validator_slots() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let validator_slot_key = builder
        .get_contract(builder.get_auction_contract_hash())
        .expect("auction should exist")
        .named_keys()[VALIDATOR_SLOTS_KEY];

    let before_validator_slots: u32 = builder
        .query(None, validator_slot_key, &[])
        .expect("should have validator slots")
        .as_cl_value()
        .expect("should be CLValue")
        .clone()
        .into_t()
        .expect("should be u32");

    let new_validator_slots = before_validator_slots + 1;

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_validator_slots(new_validator_slots)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let after_validator_slots: u32 = builder
        .query(None, validator_slot_key, &[])
        .expect("should have validator slots")
        .as_cl_value()
        .expect("should be CLValue")
        .clone()
        .into_t()
        .expect("should be u32");

    assert_eq!(
        new_validator_slots, after_validator_slots,
        "should have upgraded validator slots to expected value"
    )
}

#[ignore]
#[test]
fn should_upgrade_only_auction_delay() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let auction_delay_key = builder
        .get_contract(builder.get_auction_contract_hash())
        .expect("auction should exist")
        .named_keys()[AUCTION_DELAY_KEY];

    let before_auction_delay: u64 = builder
        .query(None, auction_delay_key, &[])
        .expect("should have auction delay")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    let new_auction_delay = before_auction_delay + 1;

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_auction_delay(new_auction_delay)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let after_auction_delay: u64 = builder
        .query(None, auction_delay_key, &[])
        .expect("should have auction delay")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    assert_eq!(
        new_auction_delay, after_auction_delay,
        "should hae upgrade version auction delay"
    )
}

#[ignore]
#[test]
fn should_upgrade_only_locked_funds_period() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let locked_funds_period_key = builder
        .get_contract(builder.get_auction_contract_hash())
        .expect("auction should exist")
        .named_keys()[LOCKED_FUNDS_PERIOD_KEY];

    let before_locked_funds_period_millis: u64 = builder
        .query(None, locked_funds_period_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    let new_locked_funds_period_millis = before_locked_funds_period_millis + 1;

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_locked_funds_period_millis(new_locked_funds_period_millis)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let after_locked_funds_period_millis: u64 = builder
        .query(None, locked_funds_period_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    assert_eq!(
        new_locked_funds_period_millis, after_locked_funds_period_millis,
        "Should have upgraded locked funds period"
    )
}

#[ignore]
#[test]
fn should_upgrade_only_round_seigniorage_rate() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let round_seigniorage_rate_key = builder
        .get_contract(builder.get_mint_contract_hash())
        .expect("auction should exist")
        .named_keys()[ROUND_SEIGNIORAGE_RATE_KEY];

    let before_round_seigniorage_rate: Ratio<U512> = builder
        .query(None, round_seigniorage_rate_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    let new_round_seigniorage_rate = Ratio::new(1, 1_000_000_000);

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_round_seigniorage_rate(new_round_seigniorage_rate)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let after_round_seigniorage_rate: Ratio<U512> = builder
        .query(None, round_seigniorage_rate_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    assert_ne!(before_round_seigniorage_rate, after_round_seigniorage_rate);

    let expected_round_seigniorage_rate = Ratio::new(
        U512::from(*new_round_seigniorage_rate.numer()),
        U512::from(*new_round_seigniorage_rate.denom()),
    );

    assert_eq!(
        expected_round_seigniorage_rate, after_round_seigniorage_rate,
        "Should have upgraded locked funds period"
    );
}

#[ignore]
#[test]
fn should_upgrade_only_unbonding_delay() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let unbonding_delay_key = builder
        .get_contract(builder.get_auction_contract_hash())
        .expect("auction should exist")
        .named_keys()[UNBONDING_DELAY_KEY];

    let before_unbonding_delay: u64 = builder
        .query(None, unbonding_delay_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    let new_unbonding_delay = DEFAULT_UNBONDING_DELAY + 5;

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_unbonding_delay(new_unbonding_delay)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let after_unbonding_delay: u64 = builder
        .query(None, unbonding_delay_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    assert_ne!(before_unbonding_delay, new_unbonding_delay);

    assert_eq!(
        new_unbonding_delay, after_unbonding_delay,
        "Should have upgraded locked funds period"
    );
}

#[ignore]
#[test]
fn should_apply_global_state_upgrade() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    // We'll try writing directly to this key.
    let unbonding_delay_key = builder
        .get_contract(builder.get_auction_contract_hash())
        .expect("auction should exist")
        .named_keys()[UNBONDING_DELAY_KEY];

    let before_unbonding_delay: u64 = builder
        .query(None, unbonding_delay_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    let new_unbonding_delay = DEFAULT_UNBONDING_DELAY + 5;

    let mut update_map = BTreeMap::new();
    update_map.insert(
        unbonding_delay_key,
        StoredValue::from(CLValue::from_t(new_unbonding_delay).expect("should create a CLValue")),
    );

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_global_state_update(update_map)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    let after_unbonding_delay: u64 = builder
        .query(None, unbonding_delay_key, &[])
        .expect("should have locked funds period")
        .as_cl_value()
        .expect("should be a CLValue")
        .clone()
        .into_t()
        .expect("should be u64");

    assert_ne!(before_unbonding_delay, new_unbonding_delay);

    assert_eq!(
        new_unbonding_delay, after_unbonding_delay,
        "Should have modified locked funds period"
    );
}

#[ignore]
#[test]
fn should_increase_max_associated_keys_after_upgrade() {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major, sem_ver.minor, sem_ver.patch + 1);

    let new_core_config = CoreConfig::new(DEFAULT_MAX_ASSOCIATED_KEYS + 1);

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_core_config(new_core_config)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    for n in (0..DEFAULT_MAX_ASSOCIATED_KEYS).map(U256::from) {
        let account_hash = {
            let mut addr = AccountHashBytes::default();
            n.to_big_endian(&mut addr);
            AccountHash::new(addr)
        };

        let add_request = ExecuteRequestBuilder::standard(
            *DEFAULT_ACCOUNT_ADDR,
            "add_update_associated_key.wasm",
            runtime_args! {
                ARG_ACCOUNT => account_hash,
            },
        )
        .with_protocol_version(new_protocol_version)
        .build();

        builder.exec(add_request).expect_success().commit();
    }

    let account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should get account");

    assert!(account.associated_keys().len() > DEFAULT_MAX_ASSOCIATED_KEYS as usize);
    assert_eq!(
        account.associated_keys().len(),
        new_core_config.max_associated_keys() as usize
    );
}
