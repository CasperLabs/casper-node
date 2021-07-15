use std::convert::TryInto;

use casper_engine_test_support::{
    internal::{
        DeployItemBuilder, ExecuteRequestBuilder, InMemoryWasmTestBuilder, DEFAULT_PAYMENT,
        DEFAULT_RUN_GENESIS_REQUEST, SYSTEM_ADDR,
    },
    DEFAULT_ACCOUNT_ADDR, MINIMUM_ACCOUNT_CREATION_BALANCE,
};
use casper_execution_engine::shared::account::Account;
use casper_types::{
    account::AccountHash, runtime_args, system::handle_payment, Key, RuntimeArgs, URef, U512,
};

const CONTRACT_FINALIZE_PAYMENT: &str = "finalize_payment.wasm";
const CONTRACT_TRANSFER_PURSE_TO_ACCOUNT: &str = "transfer_purse_to_account.wasm";
const FINALIZE_PAYMENT: &str = "finalize_payment.wasm";
const LOCAL_REFUND_PURSE: &str = "local_refund_purse";

const CREATE_PURSE_01: &str = "create_purse_01.wasm";
const ARG_PURSE_NAME: &str = "purse_name";

const ACCOUNT_ADDR: AccountHash = AccountHash::new([1u8; 32]);
pub const ARG_AMOUNT: &str = "amount";
pub const ARG_AMOUNT_SPENT: &str = "amount_spent";
pub const ARG_REFUND_FLAG: &str = "refund";
pub const ARG_ACCOUNT_KEY: &str = "account";
pub const ARG_TARGET: &str = "target";

fn initialize() -> InMemoryWasmTestBuilder {
    let mut builder = InMemoryWasmTestBuilder::default();

    let exec_request_1 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNT,
        runtime_args! {
            ARG_TARGET => *SYSTEM_ADDR,
            ARG_AMOUNT => U512::from(MINIMUM_ACCOUNT_CREATION_BALANCE)
        },
    )
    .build();

    let exec_request_2 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNT,
        runtime_args! { ARG_TARGET => ACCOUNT_ADDR, ARG_AMOUNT => U512::from(MINIMUM_ACCOUNT_CREATION_BALANCE) },
    )
    .build();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder.exec(exec_request_1).expect_success().commit();

    builder.exec(exec_request_2).expect_success().commit();

    builder
}

#[ignore]
#[test]
fn finalize_payment_should_not_be_run_by_non_system_accounts() {
    let mut builder = initialize();
    let payment_amount = U512::from(300);
    let spent_amount = U512::from(75);
    let refund_purse: Option<URef> = None;
    let args = runtime_args! {
        ARG_AMOUNT => payment_amount,
        ARG_REFUND_FLAG => refund_purse,
        ARG_AMOUNT_SPENT => Some(spent_amount),
        ARG_ACCOUNT_KEY => Some(ACCOUNT_ADDR),
    };

    let exec_request_1 = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_FINALIZE_PAYMENT,
        args.clone(),
    )
    .build();
    let exec_request_2 =
        ExecuteRequestBuilder::standard(ACCOUNT_ADDR, CONTRACT_FINALIZE_PAYMENT, args).build();

    assert!(builder.exec(exec_request_1).is_error());

    assert!(builder.exec(exec_request_2).is_error());
}

#[ignore]
#[test]
fn finalize_payment_should_refund_to_specified_purse() {
    let mut builder = InMemoryWasmTestBuilder::default();
    let payment_amount = *DEFAULT_PAYMENT;
    let refund_purse_flag: u8 = 1;
    // Don't need to run finalize_payment manually, it happens during
    // the deploy because payment code is enabled.
    let args = runtime_args! {
        ARG_AMOUNT => payment_amount,
        ARG_REFUND_FLAG => refund_purse_flag,
        ARG_AMOUNT_SPENT => Option::<U512>::None,
        ARG_ACCOUNT_KEY => Option::<AccountHash>::None,
        ARG_PURSE_NAME => LOCAL_REFUND_PURSE,
    };

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    let create_purse_request = {
        ExecuteRequestBuilder::standard(
            *DEFAULT_ACCOUNT_ADDR,
            CREATE_PURSE_01,
            runtime_args! {
                ARG_PURSE_NAME => LOCAL_REFUND_PURSE,
            },
        )
        .build()
    };

    builder.exec(create_purse_request).expect_success().commit();

    let rewards_pre_balance = builder.get_proposer_purse_balance();

    let payment_pre_balance = get_handle_payment_payment_purse_balance(&builder);
    let refund_pre_balance =
        get_named_account_balance(&builder, *DEFAULT_ACCOUNT_ADDR, LOCAL_REFUND_PURSE)
            .unwrap_or_else(U512::zero);

    assert!(
        get_handle_payment_refund_purse(&builder).is_none(),
        "refund_purse should start unset"
    );
    assert!(
        payment_pre_balance.is_zero(),
        "payment purse should start with zero balance"
    );

    let exec_request = {
        let genesis_account_hash = *DEFAULT_ACCOUNT_ADDR;

        let deploy = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_deploy_hash([1; 32])
            .with_session_code("do_nothing.wasm", RuntimeArgs::default())
            .with_payment_code(FINALIZE_PAYMENT, args)
            .with_authorization_keys(&[genesis_account_hash])
            .build();

        ExecuteRequestBuilder::new().push_deploy(deploy).build()
    };

    let proposer_reward_starting_balance = builder.get_proposer_purse_balance();

    builder.exec(exec_request).expect_success().commit();

    let transaction_fee = builder.get_proposer_purse_balance() - proposer_reward_starting_balance;

    let payment_post_balance = get_handle_payment_payment_purse_balance(&builder);
    let rewards_post_balance = builder.get_proposer_purse_balance();
    let refund_post_balance =
        get_named_account_balance(&builder, *DEFAULT_ACCOUNT_ADDR, LOCAL_REFUND_PURSE)
            .expect("should have refund balance");
    let expected_amount = rewards_pre_balance + transaction_fee;
    assert_eq!(
        expected_amount, rewards_post_balance,
        "validators should get paid; expected: {}, actual: {}",
        expected_amount, rewards_post_balance
    );

    // user gets refund
    assert_eq!(
        refund_pre_balance + payment_amount - transaction_fee,
        refund_post_balance,
        "user should get refund"
    );

    assert!(
        get_handle_payment_refund_purse(&builder).is_none(),
        "refund_purse always ends unset"
    );
    assert!(
        payment_post_balance.is_zero(),
        "payment purse should ends with zero balance"
    );
}

// ------------- utility functions -------------------- //

fn get_handle_payment_payment_purse_balance(builder: &InMemoryWasmTestBuilder) -> U512 {
    let purse = get_payment_purse_by_name(builder, handle_payment::PAYMENT_PURSE_KEY)
        .expect("should find handle payment payment purse");
    builder.get_purse_balance(purse)
}

fn get_handle_payment_refund_purse(builder: &InMemoryWasmTestBuilder) -> Option<Key> {
    let handle_payment_contract = builder.get_handle_payment_contract();
    handle_payment_contract
        .named_keys()
        .get(handle_payment::REFUND_PURSE_KEY)
        .cloned()
}

fn get_payment_purse_by_name(builder: &InMemoryWasmTestBuilder, purse_name: &str) -> Option<URef> {
    let handle_payment_contract = builder.get_handle_payment_contract();
    handle_payment_contract
        .named_keys()
        .get(purse_name)
        .and_then(Key::as_uref)
        .cloned()
}

fn get_named_account_balance(
    builder: &InMemoryWasmTestBuilder,
    account_address: AccountHash,
    name: &str,
) -> Option<U512> {
    let account_key = Key::Account(account_address);

    let account: Account = builder
        .query(None, account_key, &[])
        .and_then(|v| v.try_into().map_err(|error| format!("{:?}", error)))
        .expect("should find balance uref");

    let purse = account
        .named_keys()
        .get(name)
        .and_then(Key::as_uref)
        .cloned();

    purse.map(|uref| builder.get_purse_balance(uref))
}
