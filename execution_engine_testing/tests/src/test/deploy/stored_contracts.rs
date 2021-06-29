use std::collections::BTreeMap;

use casper_engine_test_support::{
    internal::{
        AdditiveMapDiff, DeployItemBuilder, ExecuteRequestBuilder, InMemoryWasmTestBuilder,
        UpgradeRequestBuilder, WasmTestBuilder, DEFAULT_ACCOUNT_KEY, DEFAULT_PAYMENT,
        DEFAULT_RUN_GENESIS_REQUEST,
    },
    DEFAULT_ACCOUNT_ADDR, DEFAULT_ACCOUNT_INITIAL_BALANCE,
};
use casper_execution_engine::{
    shared::{account::Account, stored_value::StoredValue, transform::Transform},
    storage::global_state::in_memory::InMemoryGlobalState,
};
use casper_types::{
    account::AccountHash,
    contracts::{ContractVersion, CONTRACT_INITIAL_VERSION, DEFAULT_ENTRY_POINT_NAME},
    runtime_args, ContractHash, EraId, Key, ProtocolVersion, RuntimeArgs, U512,
};

const ACCOUNT_1_ADDR: AccountHash = AccountHash::new([42u8; 32]);
const DEFAULT_ACTIVATION_POINT: EraId = EraId::new(1);
const DO_NOTHING_NAME: &str = "do_nothing";
const DO_NOTHING_CONTRACT_PACKAGE_HASH_NAME: &str = "do_nothing_package_hash";
const DO_NOTHING_CONTRACT_HASH_NAME: &str = "do_nothing_hash";
const INITIAL_VERSION: ContractVersion = CONTRACT_INITIAL_VERSION;
const ENTRY_FUNCTION_NAME: &str = "delegate";
const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V1_0_0;
const STORED_PAYMENT_CONTRACT_NAME: &str = "test_payment_stored.wasm";
const STORED_PAYMENT_CONTRACT_HASH_NAME: &str = "test_payment_hash";
const STORED_PAYMENT_CONTRACT_PACKAGE_HASH_NAME: &str = "test_payment_package_hash";
const PAY: &str = "pay";
const TRANSFER: &str = "transfer";
const TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME: &str = "transfer_purse_to_account";
const TRANSFER_PURSE_TO_ACCOUNT_STORED_HASH_KEY_NAME: &str = "transfer_purse_to_account_hash";
// Currently Error enum that holds this variant is private and can't be used otherwise to compare
// message
const EXPECTED_ERROR_MESSAGE: &str = "IncompatibleProtocolMajorVersion { expected: 2, actual: 1 }";
const EXPECTED_VERSION_ERROR_MESSAGE: &str = "InvalidContractVersion(ContractVersionKey(2, 1))";

const ARG_TARGET: &str = "target";
const ARG_AMOUNT: &str = "amount";

/// Prepares a upgrade request with pre-loaded deploy code, and new protocol version.
fn make_upgrade_request(new_protocol_version: ProtocolVersion) -> UpgradeRequestBuilder {
    UpgradeRequestBuilder::new()
        .with_current_protocol_version(PROTOCOL_VERSION)
        .with_new_protocol_version(new_protocol_version)
        .with_activation_point(DEFAULT_ACTIVATION_POINT)
}

fn store_payment_to_account_context(
    builder: &mut WasmTestBuilder<InMemoryGlobalState>,
) -> (Account, ContractHash) {
    // store payment contract
    let exec_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        STORED_PAYMENT_CONTRACT_NAME,
        RuntimeArgs::default(),
    )
    .build();

    builder.exec_commit_finish(exec_request);

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should have account");

    // check account named keys
    let hash = default_account
        .named_keys()
        .get(STORED_PAYMENT_CONTRACT_PACKAGE_HASH_NAME)
        .expect("key should exist")
        .into_hash()
        .expect("should be a hash")
        .into();

    (default_account, hash)
}

#[ignore]
#[test]
fn should_exec_non_stored_code() {
    // using the new execute logic, passing code for both payment and session
    // should work exactly as it did with the original exec logic

    let account_1_account_hash = ACCOUNT_1_ADDR;
    let payment_purse_amount = *DEFAULT_PAYMENT;
    let transferred_amount = 1;

    let exec_request = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_session_code(
                &format!("{}.wasm", TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME),
                runtime_args! {
                    ARG_TARGET => account_1_account_hash,
                    ARG_AMOUNT => U512::from(transferred_amount)
                },
            )
            .with_empty_payment_bytes(runtime_args! {
                ARG_AMOUNT => payment_purse_amount,
            })
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([1; 32])
            .build();

        ExecuteRequestBuilder::new().push_deploy(deploy).build()
    };

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let proposer_reward_starting_balance = builder.get_proposer_purse_balance();

    builder.exec(exec_request).expect_success().commit();

    let transaction_fee = builder.get_proposer_purse_balance() - proposer_reward_starting_balance;

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should get genesis account");
    let modified_balance: U512 = builder.get_purse_balance(default_account.main_purse());

    let initial_balance: U512 = U512::from(DEFAULT_ACCOUNT_INITIAL_BALANCE);

    assert_ne!(
        modified_balance, initial_balance,
        "balance should be less than initial balance"
    );

    let tally = transaction_fee + U512::from(transferred_amount) + modified_balance;

    assert_eq!(
        initial_balance, tally,
        "no net resources should be gained or lost post-distribution"
    );
}

#[ignore]
#[test]
fn should_exec_stored_code_by_hash() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // genesis
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    // store payment
    let proposer_reward_starting_balance_alpha = builder.get_proposer_purse_balance();

    let (default_account, hash) = store_payment_to_account_context(&mut builder);

    // verify stored contract functions as expected by checking all the maths

    let (motes_alpha, modified_balance_alpha) = {
        // get modified balance
        let modified_balance_alpha: U512 = builder.get_purse_balance(default_account.main_purse());

        let transaction_fee_alpha =
            builder.get_proposer_purse_balance() - proposer_reward_starting_balance_alpha;
        (transaction_fee_alpha, modified_balance_alpha)
    };

    let transferred_amount = 1;

    // next make another deploy that USES stored payment logic

    let proposer_reward_starting_balance_bravo = builder.get_proposer_purse_balance();

    {
        let exec_request_stored_payment = {
            let account_1_account_hash = ACCOUNT_1_ADDR;
            let deploy = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_session_code(
                    &format!("{}.wasm", TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME),
                    runtime_args! { ARG_TARGET => account_1_account_hash, ARG_AMOUNT => U512::from(transferred_amount) },
                )
                .with_stored_versioned_payment_contract_by_hash(
                    hash.value(),
                    Some(CONTRACT_INITIAL_VERSION),
                    PAY,
                    runtime_args! {
                        ARG_AMOUNT => payment_purse_amount,
                    },
                )
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash([2; 32])
                .build();

            ExecuteRequestBuilder::new().push_deploy(deploy).build()
        };

        builder.exec_commit_finish(exec_request_stored_payment);
    }

    let (motes_bravo, modified_balance_bravo) = {
        let modified_balance_bravo: U512 = builder.get_purse_balance(default_account.main_purse());

        let transaction_fee_bravo =
            builder.get_proposer_purse_balance() - proposer_reward_starting_balance_bravo;

        (transaction_fee_bravo, modified_balance_bravo)
    };

    let initial_balance: U512 = U512::from(DEFAULT_ACCOUNT_INITIAL_BALANCE);

    assert!(
        modified_balance_alpha < initial_balance,
        "balance should be less than initial balance"
    );

    assert!(
        modified_balance_bravo < modified_balance_alpha,
        "second modified balance should be less than first modified balance"
    );

    let tally = motes_alpha + motes_bravo + U512::from(transferred_amount) + modified_balance_bravo;

    assert_eq!(
        initial_balance, tally,
        "no net resources should be gained or lost post-distribution"
    );
}

#[ignore]
#[test]
fn should_exec_stored_code_by_named_hash() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // genesis
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    // store payment
    let proposer_reward_starting_balance_alpha = builder.get_proposer_purse_balance();

    let (default_account, _) = store_payment_to_account_context(&mut builder);

    // verify stored contract functions as expected by checking all the maths

    let (motes_alpha, modified_balance_alpha) = {
        // get modified balance
        let modified_balance_alpha: U512 = builder.get_purse_balance(default_account.main_purse());

        // get cost
        let transaction_fee_alpha =
            builder.get_proposer_purse_balance() - proposer_reward_starting_balance_alpha;

        (transaction_fee_alpha, modified_balance_alpha)
    };

    let transferred_amount = 1;

    // next make another deploy that USES stored payment logic
    let proposer_reward_starting_balance_bravo = builder.get_proposer_purse_balance();

    {
        let exec_request_stored_payment = {
            let account_1_account_hash = ACCOUNT_1_ADDR;
            let deploy = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_session_code(
                    &format!("{}.wasm", TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME),
                    runtime_args! { ARG_TARGET => account_1_account_hash, ARG_AMOUNT => U512::from(transferred_amount) },
                )
                .with_stored_versioned_payment_contract_by_name(
                    STORED_PAYMENT_CONTRACT_PACKAGE_HASH_NAME,
                    Some(CONTRACT_INITIAL_VERSION),
                    PAY,
                    runtime_args! {
                        ARG_AMOUNT => payment_purse_amount,
                    },
                )
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash([2; 32])
                .build();

            ExecuteRequestBuilder::new().push_deploy(deploy).build()
        };

        builder.exec_commit_finish(exec_request_stored_payment);
    }

    let (motes_bravo, modified_balance_bravo) = {
        let modified_balance_bravo: U512 = builder.get_purse_balance(default_account.main_purse());

        let transaction_fee_bravo =
            builder.get_proposer_purse_balance() - proposer_reward_starting_balance_bravo;

        (transaction_fee_bravo, modified_balance_bravo)
    };

    let initial_balance: U512 = U512::from(DEFAULT_ACCOUNT_INITIAL_BALANCE);

    assert!(
        modified_balance_alpha < initial_balance,
        "balance should be less than initial balance"
    );

    assert!(
        modified_balance_bravo < modified_balance_alpha,
        "second modified balance should be less than first modified balance"
    );

    let tally = motes_alpha + motes_bravo + U512::from(transferred_amount) + modified_balance_bravo;

    assert_eq!(
        initial_balance, tally,
        "no net resources should be gained or lost post-distribution"
    );
}

#[ignore]
#[test]
fn should_exec_payment_and_session_stored_code() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // genesis
    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    // store payment
    // get cost
    let proposer_reward_starting_balance_alpha = builder.get_proposer_purse_balance();

    store_payment_to_account_context(&mut builder);

    // verify stored contract functions as expected by checking all the maths

    let motes_alpha = builder.get_proposer_purse_balance() - proposer_reward_starting_balance_alpha;

    // next store transfer contract
    let exec_request_store_transfer = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_session_code(
                &format!("{}_stored.wasm", TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME),
                RuntimeArgs::default(),
            )
            .with_stored_versioned_payment_contract_by_name(
                STORED_PAYMENT_CONTRACT_PACKAGE_HASH_NAME,
                Some(CONTRACT_INITIAL_VERSION),
                PAY,
                runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([2; 32])
            .build();

        ExecuteRequestBuilder::new().push_deploy(deploy).build()
    };

    let proposer_reward_starting_balance_bravo = builder.get_proposer_purse_balance();

    builder
        .exec(exec_request_store_transfer)
        .commit()
        .expect_success();

    let motes_bravo = builder.get_proposer_purse_balance() - proposer_reward_starting_balance_bravo;

    let transferred_amount = 1;

    // next make another deploy that USES stored payment logic & stored transfer
    // logic
    let exec_request_stored_only = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_stored_versioned_contract_by_name(
                TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME,
                Some(CONTRACT_INITIAL_VERSION),
                TRANSFER,
                runtime_args! {
                    ARG_TARGET => ACCOUNT_1_ADDR,
                    ARG_AMOUNT => U512::from(transferred_amount),
                },
            )
            .with_stored_versioned_payment_contract_by_name(
                STORED_PAYMENT_CONTRACT_PACKAGE_HASH_NAME,
                Some(CONTRACT_INITIAL_VERSION),
                PAY,
                runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([3; 32])
            .build();

        ExecuteRequestBuilder::new().push_deploy(deploy).build()
    };

    let proposer_reward_starting_balance = builder.get_proposer_purse_balance();

    builder
        .exec(exec_request_stored_only)
        .commit()
        .expect_success();

    let motes_charlie = builder.get_proposer_purse_balance() - proposer_reward_starting_balance;

    let modified_balance: U512 = {
        let default_account = builder
            .get_account(*DEFAULT_ACCOUNT_ADDR)
            .expect("should get genesis account");
        builder.get_purse_balance(default_account.main_purse())
    };

    let initial_balance: U512 = U512::from(DEFAULT_ACCOUNT_INITIAL_BALANCE);

    let tally = motes_alpha
        + motes_bravo
        + motes_charlie
        + U512::from(transferred_amount)
        + modified_balance;

    assert_eq!(
        initial_balance, tally,
        "no net resources should be gained or lost post-distribution"
    );
}

#[ignore]
#[test]
fn should_have_equivalent_transforms_with_stored_contract_pointers() {
    let account_1_account_hash = ACCOUNT_1_ADDR;
    let payment_purse_amount = *DEFAULT_PAYMENT;
    let transferred_amount = 1;

    let stored_transforms = {
        let mut builder = InMemoryWasmTestBuilder::default();

        let exec_request_1 = {
            let store_transfer = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_session_code(
                    &format!("{}_stored.wasm", TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME),
                    RuntimeArgs::default(),
                )
                .with_empty_payment_bytes(runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                })
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash([1; 32])
                .build();

            ExecuteRequestBuilder::new()
                .push_deploy(store_transfer)
                .build()
        };

        let exec_request_2 = {
            let store_transfer = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_session_code(STORED_PAYMENT_CONTRACT_NAME, RuntimeArgs::default())
                .with_empty_payment_bytes(runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                })
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash([2; 32])
                .build();

            ExecuteRequestBuilder::new()
                .push_deploy(store_transfer)
                .build()
        };

        builder
            .run_genesis(&DEFAULT_RUN_GENESIS_REQUEST)
            .exec(exec_request_1)
            .expect_success()
            .commit();

        builder.exec(exec_request_2).expect_success().commit();

        let call_stored_request = {
            let deploy = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_stored_session_named_key(
                    TRANSFER_PURSE_TO_ACCOUNT_STORED_HASH_KEY_NAME,
                    TRANSFER,
                    runtime_args! { ARG_TARGET => account_1_account_hash, ARG_AMOUNT => U512::from(transferred_amount) },
                )
                .with_stored_payment_named_key(
                    STORED_PAYMENT_CONTRACT_HASH_NAME,
                    PAY,
                    runtime_args! {
                        ARG_AMOUNT => payment_purse_amount,
                    },
                )
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash([3; 32])
                .build();

            ExecuteRequestBuilder::new().push_deploy(deploy).build()
        };

        builder
            .exec(call_stored_request)
            .expect_success()
            .commit()
            .get_transforms()[2]
            .to_owned()
    };

    let provided_transforms = {
        let do_nothing_request = |deploy_hash: [u8; 32]| {
            let deploy = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_session_code(&format!("{}.wasm", DO_NOTHING_NAME), RuntimeArgs::default())
                .with_empty_payment_bytes(runtime_args! { ARG_AMOUNT => payment_purse_amount, })
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash(deploy_hash)
                .build();

            ExecuteRequestBuilder::new().push_deploy(deploy).build()
        };

        let provided_request = {
            let deploy = DeployItemBuilder::new()
                .with_address(*DEFAULT_ACCOUNT_ADDR)
                .with_session_code(
                    &format!("{}.wasm", TRANSFER_PURSE_TO_ACCOUNT_CONTRACT_NAME),
                    runtime_args! { ARG_TARGET => account_1_account_hash, ARG_AMOUNT => U512::from(transferred_amount) },
                )
                .with_empty_payment_bytes(
                    runtime_args! {
                            ARG_AMOUNT => payment_purse_amount,
                        },
                )
                .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
                .with_deploy_hash([3; 32])
                .build();

            ExecuteRequestBuilder::new().push_deploy(deploy).build()
        };

        let mut builder = InMemoryWasmTestBuilder::default();

        builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

        builder
            .exec(do_nothing_request([1; 32]))
            .expect_success()
            .commit();
        builder
            .exec(do_nothing_request([2; 32]))
            .expect_success()
            .commit();

        builder
            .exec(provided_request)
            .expect_success()
            .get_transforms()[2]
            .to_owned()
    };

    let diff = AdditiveMapDiff::new(provided_transforms, stored_transforms);

    let left: BTreeMap<&Key, &Transform> = diff.left().iter().collect();
    let right: BTreeMap<&Key, &Transform> = diff.right().iter().collect();

    // The diff contains the same keys...
    assert!(Iterator::eq(left.keys(), right.keys()));

    // ...but a few different values
    for lr in left.values().zip(right.values()) {
        match lr {
            (
                Transform::Write(StoredValue::CLValue(l_value)),
                Transform::Write(StoredValue::CLValue(r_value)),
            ) => {
                // differing refunds and balances
                let _ = l_value.to_owned().into_t::<U512>().expect("should be U512");
                let _ = r_value.to_owned().into_t::<U512>().expect("should be U512");
            }
            (
                Transform::Write(StoredValue::Account(la)),
                Transform::Write(StoredValue::Account(ra)),
            ) => {
                assert_eq!(la.account_hash(), ra.account_hash());
                assert_eq!(la.main_purse(), ra.main_purse());
                assert_eq!(la.action_thresholds(), ra.action_thresholds());

                assert!(Iterator::eq(
                    la.associated_keys().iter(),
                    ra.associated_keys().iter(),
                ));

                // la has stored contracts under named urefs
                assert_ne!(la.named_keys(), ra.named_keys());
            }
            (
                Transform::Write(StoredValue::Transfer(l_value)),
                Transform::Write(StoredValue::Transfer(r_value)),
            ) => assert_eq!(l_value, r_value),
            (
                Transform::Write(StoredValue::DeployInfo(l_value)),
                Transform::Write(StoredValue::DeployInfo(r_value)),
            ) => {
                assert_eq!(l_value.deploy_hash, r_value.deploy_hash);
                assert_eq!(l_value.from, r_value.from);
                assert_eq!(l_value.source, r_value.source);
                assert_eq!(l_value.transfers, r_value.transfers);
                assert_ne!(l_value.gas, r_value.gas);
            }
            (Transform::AddUInt512(_), Transform::AddUInt512(_)) => {
                // differing payment
            }
            _ => {
                println!("lr: {:?}", lr);
                panic!("unexpected diff");
            }
        }
    }
}

#[ignore]
#[test]
fn should_fail_payment_stored_at_named_key_with_incompatible_major_version() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // first, store payment contract
    let exec_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        STORED_PAYMENT_CONTRACT_NAME,
        RuntimeArgs::default(),
    )
    .build();

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder.exec_commit_finish(exec_request);

    let query_result = builder
        .query(None, Key::Account(*DEFAULT_ACCOUNT_ADDR), &[])
        .expect("should query default account");
    let default_account = query_result
        .as_account()
        .expect("query result should be an account");

    assert!(
        default_account
            .named_keys()
            .contains_key(STORED_PAYMENT_CONTRACT_HASH_NAME),
        "standard_payment should be present"
    );

    //
    // upgrade with new wasm costs with modified mint for given version to avoid missing wasm costs
    // table that's queried early
    //
    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major + 1, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = make_upgrade_request(new_protocol_version).build();

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    // next make another deploy that USES stored payment logic
    let exec_request_stored_payment = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_session_code(&format!("{}.wasm", DO_NOTHING_NAME), RuntimeArgs::default())
            .with_stored_payment_named_key(
                STORED_PAYMENT_CONTRACT_HASH_NAME,
                PAY,
                runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([2; 32])
            .build();

        ExecuteRequestBuilder::new()
            .push_deploy(deploy)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    let test_result = builder.exec(exec_request_stored_payment).commit();

    assert!(
        test_result.is_error(),
        "calling a payment module with increased major protocol version should be error"
    );
    let error_message = builder
        .exec_error_message(1)
        .expect("should have exec error");
    assert!(
        error_message.contains(EXPECTED_ERROR_MESSAGE),
        "{:?}",
        error_message
    );
}

#[ignore]
#[test]
fn should_fail_payment_stored_at_hash_with_incompatible_major_version() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // first, store payment contract
    let exec_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        STORED_PAYMENT_CONTRACT_NAME,
        RuntimeArgs::default(),
    )
    .build();

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder.exec_commit_finish(exec_request);

    let query_result = builder
        .query(None, Key::Account(*DEFAULT_ACCOUNT_ADDR), &[])
        .expect("should query default account");
    let default_account = query_result
        .as_account()
        .expect("query result should be an account");
    let stored_payment_contract_hash = default_account
        .named_keys()
        .get(STORED_PAYMENT_CONTRACT_HASH_NAME)
        .expect("should have standard_payment named key")
        .into_hash()
        .expect("standard_payment should be an uref");

    //
    // upgrade with new wasm costs with modified mint for given version to avoid missing wasm costs
    // table that's queried early
    //
    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major + 1, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = make_upgrade_request(new_protocol_version).build();

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    // next make another deploy that USES stored payment logic
    let exec_request_stored_payment = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_session_code(&format!("{}.wasm", DO_NOTHING_NAME), RuntimeArgs::default())
            .with_stored_payment_hash(
                stored_payment_contract_hash.into(),
                DEFAULT_ENTRY_POINT_NAME,
                runtime_args! { ARG_AMOUNT => payment_purse_amount },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([2; 32])
            .build();

        ExecuteRequestBuilder::new()
            .push_deploy(deploy)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    let test_result = builder.exec(exec_request_stored_payment).commit();

    assert!(
        test_result.is_error(),
        "calling a payment module with increased major protocol version should be error"
    );
    let error_message = builder
        .exec_error_message(1)
        .expect("should have exec error");
    assert!(error_message.contains(EXPECTED_ERROR_MESSAGE));
}

#[ignore]
#[test]
fn should_fail_session_stored_at_named_key_with_incompatible_major_version() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // first, store payment contract for v1.0.0
    let exec_request_1 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        &format!("{}_stored.wasm", DO_NOTHING_NAME),
        RuntimeArgs::default(),
    )
    .build();

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder.exec_commit_finish(exec_request_1);

    let query_result = builder
        .query(None, Key::Account(*DEFAULT_ACCOUNT_ADDR), &[])
        .expect("should query default account");
    let default_account = query_result
        .as_account()
        .expect("query result should be an account");
    assert!(
        default_account
            .named_keys()
            .contains_key(DO_NOTHING_CONTRACT_HASH_NAME),
        "do_nothing should be present in named keys"
    );

    //
    // upgrade with new wasm costs with modified mint for given version
    //
    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major + 1, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = make_upgrade_request(new_protocol_version).build();

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    // Call stored session code

    let exec_request_stored_payment = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_stored_session_named_key(
                DO_NOTHING_CONTRACT_HASH_NAME,
                ENTRY_FUNCTION_NAME,
                RuntimeArgs::new(),
            )
            .with_payment_code(
                STORED_PAYMENT_CONTRACT_NAME,
                runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([2; 32])
            .build();

        ExecuteRequestBuilder::new()
            .push_deploy(deploy)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    let test_result = builder.exec(exec_request_stored_payment).commit();

    assert!(
        test_result.is_error(),
        "calling a session module with increased major protocol version should be error",
    );
    let error_message = builder
        .exec_error_message(1)
        .expect("should have exec error");
    assert!(
        error_message.contains(EXPECTED_ERROR_MESSAGE),
        "{:?}",
        error_message
    );
}

#[ignore]
#[test]
fn should_fail_session_stored_at_named_key_with_missing_new_major_version() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // first, store payment contract for v1.0.0
    let exec_request_1 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        &format!("{}_stored.wasm", DO_NOTHING_NAME),
        RuntimeArgs::default(),
    )
    .build();

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder.exec_commit_finish(exec_request_1);

    let query_result = builder
        .query(None, Key::Account(*DEFAULT_ACCOUNT_ADDR), &[])
        .expect("should query default account");
    let default_account = query_result
        .as_account()
        .expect("query result should be an account");
    assert!(
        default_account
            .named_keys()
            .contains_key(DO_NOTHING_CONTRACT_HASH_NAME),
        "do_nothing should be present in named keys"
    );

    //
    // upgrade with new wasm costs with modified mint for given version
    //
    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major + 1, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = make_upgrade_request(new_protocol_version).build();

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    // Call stored session code

    let exec_request_stored_payment = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_stored_versioned_contract_by_name(
                DO_NOTHING_CONTRACT_PACKAGE_HASH_NAME,
                Some(INITIAL_VERSION),
                ENTRY_FUNCTION_NAME,
                RuntimeArgs::new(),
            )
            .with_payment_code(
                STORED_PAYMENT_CONTRACT_NAME,
                runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([2; 32])
            .build();

        ExecuteRequestBuilder::new()
            .push_deploy(deploy)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    let test_result = builder.exec(exec_request_stored_payment).commit();

    assert!(
        test_result.is_error(),
        "calling a session module with increased major protocol version should be error",
    );
    let error_message = builder
        .exec_error_message(1)
        .expect("should have exec error");
    assert!(
        error_message.contains(EXPECTED_VERSION_ERROR_MESSAGE),
        "{:?}",
        error_message
    );
}

#[ignore]
#[test]
fn should_fail_session_stored_at_hash_with_incompatible_major_version() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    // first, store payment contract for v1.0.0
    let exec_request_1 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        &format!("{}_stored.wasm", DO_NOTHING_NAME),
        RuntimeArgs::default(),
    )
    .build();

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder.exec_commit_finish(exec_request_1);

    //
    // upgrade with new wasm costs with modified mint for given version
    //
    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major + 1, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = make_upgrade_request(new_protocol_version).build();

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    // Call stored session code

    let exec_request_stored_payment = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_stored_session_named_key(
                DO_NOTHING_CONTRACT_HASH_NAME,
                ENTRY_FUNCTION_NAME,
                RuntimeArgs::new(),
            )
            .with_payment_code(
                STORED_PAYMENT_CONTRACT_NAME,
                runtime_args! {
                    ARG_AMOUNT => payment_purse_amount,
                },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([2; 32])
            .build();

        ExecuteRequestBuilder::new()
            .push_deploy(deploy)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    let test_result = builder.exec(exec_request_stored_payment).commit();

    assert!(
        test_result.is_error(),
        "calling a session module with increased major protocol version should be error",
    );
    let error_message = builder
        .exec_error_message(1)
        .expect("should have exec error");
    assert!(
        error_message.contains(EXPECTED_ERROR_MESSAGE),
        "{:?}",
        error_message
    );
}

#[ignore]
#[test]
fn should_execute_stored_payment_and_session_code_with_new_major_version() {
    let payment_purse_amount = *DEFAULT_PAYMENT;

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    //
    // upgrade with new wasm costs with modified mint for given version
    //
    let sem_ver = PROTOCOL_VERSION.value();
    let new_protocol_version =
        ProtocolVersion::from_parts(sem_ver.major + 1, sem_ver.minor, sem_ver.patch);

    let mut upgrade_request = make_upgrade_request(new_protocol_version).build();

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request)
        .expect_upgrade_success();

    // first, store payment contract for v2.0.0

    let exec_request_1 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        STORED_PAYMENT_CONTRACT_NAME,
        RuntimeArgs::default(),
    )
    .with_protocol_version(new_protocol_version)
    .build();

    let exec_request_2 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        &format!("{}_stored.wasm", DO_NOTHING_NAME),
        RuntimeArgs::default(),
    )
    .with_protocol_version(new_protocol_version)
    .build();

    // store both contracts
    builder.exec(exec_request_1).expect_success().commit();

    let test_result = builder
        .exec(exec_request_2)
        .expect_success()
        .commit()
        .finish();

    // query both stored contracts by their named keys
    let query_result = test_result
        .builder()
        .query(None, Key::Account(*DEFAULT_ACCOUNT_ADDR), &[])
        .expect("should query default account");
    let default_account = query_result
        .as_account()
        .expect("query result should be an account");
    let test_payment_stored_hash = default_account
        .named_keys()
        .get(STORED_PAYMENT_CONTRACT_HASH_NAME)
        .expect("standard_payment should be present in named keys")
        .into_hash()
        .expect("standard_payment named key should be hash");

    let exec_request_stored_payment = {
        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_stored_versioned_contract_by_name(
                DO_NOTHING_CONTRACT_PACKAGE_HASH_NAME,
                Some(INITIAL_VERSION),
                ENTRY_FUNCTION_NAME,
                RuntimeArgs::new(),
            )
            .with_stored_payment_hash(
                test_payment_stored_hash.into(),
                "pay",
                runtime_args! { ARG_AMOUNT => payment_purse_amount },
            )
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_KEY])
            .with_deploy_hash([3; 32])
            .build();

        ExecuteRequestBuilder::new()
            .push_deploy(deploy)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    InMemoryWasmTestBuilder::from_result(test_result)
        .exec(exec_request_stored_payment)
        .expect_success()
        .commit();
}
