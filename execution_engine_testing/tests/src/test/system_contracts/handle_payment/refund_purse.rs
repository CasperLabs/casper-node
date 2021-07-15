use casper_engine_test_support::{
    internal::{
        DeployItemBuilder, ExecuteRequestBuilder, InMemoryWasmTestBuilder, DEFAULT_PAYMENT,
        DEFAULT_RUN_GENESIS_REQUEST,
    },
    DEFAULT_ACCOUNT_ADDR, MINIMUM_ACCOUNT_CREATION_BALANCE,
};
use casper_types::{account::AccountHash, runtime_args, RuntimeArgs, U512};

const CONTRACT_TRANSFER_PURSE_TO_ACCOUNT: &str = "transfer_purse_to_account.wasm";
const ACCOUNT_1_ADDR: AccountHash = AccountHash::new([1u8; 32]);
const ARG_PAYMENT_AMOUNT: &str = "payment_amount";
const CREATE_PURSE_01: &str = "create_purse_01.wasm";
const ARG_PURSE_NAME: &str = "purse_name";
const ARG_PURSE_NAME_1: &str = "purse_name_1";
const ARG_PURSE_NAME_2: &str = "purse_name_2";
const LOCAL_REFUND_PURSE_1: &str = "local_refund_purse_1";
const LOCAL_REFUND_PURSE_2: &str = "local_refund_purse_2";

#[ignore]
#[test]
fn should_run_refund_purse_contract_default_account() {
    let mut builder = initialize();
    refund_tests(&mut builder, *DEFAULT_ACCOUNT_ADDR);
}

#[ignore]
#[test]
fn should_run_refund_purse_contract_account_1() {
    let mut builder = initialize();
    transfer(
        &mut builder,
        ACCOUNT_1_ADDR,
        U512::from(MINIMUM_ACCOUNT_CREATION_BALANCE),
    );
    refund_tests(&mut builder, ACCOUNT_1_ADDR);
}

fn initialize() -> InMemoryWasmTestBuilder {
    let mut builder = InMemoryWasmTestBuilder::default();

    builder.run_genesis(&DEFAULT_RUN_GENESIS_REQUEST);

    builder
}

fn transfer(builder: &mut InMemoryWasmTestBuilder, account_hash: AccountHash, amount: U512) {
    let exec_request = {
        ExecuteRequestBuilder::standard(
            *DEFAULT_ACCOUNT_ADDR,
            CONTRACT_TRANSFER_PURSE_TO_ACCOUNT,
            runtime_args! {
                "target" => account_hash,
                "amount" => amount,
            },
        )
        .build()
    };

    builder.exec(exec_request).expect_success().commit();
}

fn refund_tests(builder: &mut InMemoryWasmTestBuilder, account_hash: AccountHash) {
    let create_purse_request_1 = {
        ExecuteRequestBuilder::standard(
            account_hash,
            CREATE_PURSE_01,
            runtime_args! {
                ARG_PURSE_NAME => LOCAL_REFUND_PURSE_1,
            },
        )
        .build()
    };

    let create_purse_request_2 = {
        ExecuteRequestBuilder::standard(
            account_hash,
            CREATE_PURSE_01,
            runtime_args! {
                ARG_PURSE_NAME => LOCAL_REFUND_PURSE_2,
            },
        )
        .build()
    };

    builder
        .exec(create_purse_request_1)
        .expect_success()
        .commit();
    builder
        .exec(create_purse_request_2)
        .expect_success()
        .commit();

    let refund_purse_request = {
        let deploy = DeployItemBuilder::new()
            .with_address(account_hash)
            .with_deploy_hash([2; 32])
            .with_session_code("do_nothing.wasm", RuntimeArgs::default())
            .with_payment_code(
                "refund_purse.wasm",
                runtime_args! {
                    ARG_PAYMENT_AMOUNT => *DEFAULT_PAYMENT,
                    ARG_PURSE_NAME_1 => LOCAL_REFUND_PURSE_1,
                    ARG_PURSE_NAME_2 => LOCAL_REFUND_PURSE_2,
                },
            )
            .with_authorization_keys(&[account_hash])
            .build();

        ExecuteRequestBuilder::new().push_deploy(deploy).build()
    };

    builder.exec(refund_purse_request).expect_success().commit();
}
