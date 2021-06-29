use casper_engine_test_support::{
    internal::{
        DeployItemBuilder, ExecuteRequestBuilder, InMemoryWasmTestBuilder, UpgradeRequestBuilder,
        DEFAULT_PAYMENT, DEFAULT_PROTOCOL_VERSION, DEFAULT_RUN_GENESIS_REQUEST,
    },
    DEFAULT_ACCOUNT_ADDR,
};
use casper_execution_engine::{
    core::{
        engine_state::{Error as CoreError, WASMLESS_TRANSFER_FIXED_GAS_PRICE},
        execution::Error as ExecError,
    },
    shared::{
        gas::Gas,
        motes::Motes,
        system_costs::{
            auction_costs::AuctionCosts, handle_payment_costs::HandlePaymentCosts,
            mint_costs::MintCosts, standard_payment_costs::StandardPaymentCosts, SystemCosts,
        },
    },
    storage::protocol_data::DEFAULT_WASMLESS_TRANSFER_COST,
};
use casper_types::{
    account::AccountHash,
    runtime_args,
    system::{handle_payment, mint},
    AccessRights, ApiError, EraId, Key, ProtocolVersion, RuntimeArgs, URef, U512,
};

const CONTRACT_TRANSFER_PURSE_TO_ACCOUNT: &str = "transfer_purse_to_account.wasm";
const CONTRACT_CREATE_PURSE_01: &str = "create_purse_01.wasm";
const TRANSFER_RESULT_NAMED_KEY: &str = "transfer_result";
const TEST_PURSE_NAME: &str = "test_purse";
const ARG_PURSE_NAME: &str = "purse_name";

const ACCOUNT_1_ADDR: AccountHash = AccountHash::new([1u8; 32]);
const ACCOUNT_2_ADDR: AccountHash = AccountHash::new([2u8; 32]);

#[ignore]
#[test]
fn should_transfer_wasmless_account_to_purse() {
    transfer_wasmless(WasmlessTransfer::AccountMainPurseToPurse);
}

#[ignore]
#[test]
fn should_transfer_wasmless_account_to_account() {
    transfer_wasmless(WasmlessTransfer::AccountMainPurseToAccountMainPurse);
}

#[ignore]
#[test]
fn should_transfer_wasmless_account_to_account_by_key() {
    transfer_wasmless(WasmlessTransfer::AccountToAccountByKey);
}

#[ignore]
#[test]
fn should_transfer_wasmless_purse_to_purse() {
    transfer_wasmless(WasmlessTransfer::PurseToPurse);
}

#[ignore]
#[test]
fn should_transfer_wasmless_amount_as_u64() {
    transfer_wasmless(WasmlessTransfer::AmountAsU64);
}

enum WasmlessTransfer {
    AccountMainPurseToPurse,
    AccountMainPurseToAccountMainPurse,
    PurseToPurse,
    AccountToAccountByKey,
    AmountAsU64,
}

fn transfer_wasmless(wasmless_transfer: WasmlessTransfer) {
    let create_account_2: bool = true;
    let mut builder = init_wasmless_transform_builder(create_account_2);
    let transfer_amount: U512 = U512::from(1000);
    let id: Option<u64> = None;

    let account_1_purse = builder
        .get_account(ACCOUNT_1_ADDR)
        .expect("should get account 1")
        .main_purse();

    let account_2_purse = builder
        .get_account(ACCOUNT_2_ADDR)
        .expect("should get account 2")
        .main_purse();

    let account_1_starting_balance = builder.get_purse_balance(account_1_purse);
    let account_2_starting_balance = builder.get_purse_balance(account_2_purse);

    let runtime_args = match wasmless_transfer {
        WasmlessTransfer::AccountMainPurseToPurse => {
            runtime_args! {
                mint::ARG_TARGET => account_2_purse,
                mint::ARG_AMOUNT => transfer_amount,
                mint::ARG_ID => id
            }
        }
        WasmlessTransfer::AccountMainPurseToAccountMainPurse => {
            runtime_args! {
                mint::ARG_TARGET => ACCOUNT_2_ADDR,
                mint::ARG_AMOUNT => transfer_amount,
                mint::ARG_ID => id
            }
        }
        WasmlessTransfer::AccountToAccountByKey => {
            runtime_args! {
                mint::ARG_TARGET => Key::Account(ACCOUNT_2_ADDR),
                mint::ARG_AMOUNT => transfer_amount,
                mint::ARG_ID => id
            }
        }
        WasmlessTransfer::PurseToPurse => {
            runtime_args! {
                mint::ARG_SOURCE => account_1_purse,
                mint::ARG_TARGET => account_2_purse,
                mint::ARG_AMOUNT => transfer_amount,
                mint::ARG_ID => id
            }
        }
        WasmlessTransfer::AmountAsU64 => {
            runtime_args! {
                mint::ARG_SOURCE => account_1_purse,
                mint::ARG_TARGET => account_2_purse,
                mint::ARG_AMOUNT => 1000u64,
                mint::ARG_ID => id
            }
        }
    };

    let no_wasm_transfer_request = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_1_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_1_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder
        .exec(no_wasm_transfer_request)
        .expect_success()
        .commit();

    let wasmless_transfer_gas_cost = Gas::from(DEFAULT_WASMLESS_TRANSFER_COST);
    let wasmless_transfer_cost = Motes::from_gas(
        wasmless_transfer_gas_cost,
        WASMLESS_TRANSFER_FIXED_GAS_PRICE,
    )
    .expect("gas overflow");

    assert_eq!(
        account_1_starting_balance - transfer_amount - wasmless_transfer_cost.value(),
        builder.get_purse_balance(account_1_purse),
        "account 1 ending balance incorrect"
    );
    assert_eq!(
        account_2_starting_balance + transfer_amount,
        builder.get_purse_balance(account_2_purse),
        "account 2 ending balance incorrect"
    );

    // Make sure postconditions are met: payment purse has to be empty after finalization
    let handle_payment = builder.get_handle_payment_contract_hash();
    let contract = builder
        .get_contract(handle_payment)
        .expect("should have contract");
    let key = contract
        .named_keys()
        .get(handle_payment::PAYMENT_PURSE_KEY)
        .cloned()
        .expect("should have named key");

    assert_eq!(
        builder.get_purse_balance(key.into_uref().unwrap()),
        U512::zero()
    );
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_to_self_by_addr() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::TransferToSelfByAddr);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_to_self_by_key() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::TransferToSelfByKey);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_to_self_by_uref() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::TransferToSelfByURef);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_other_account_by_addr() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::OtherSourceAccountByAddr);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_other_account_by_key() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::OtherSourceAccountByKey);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_other_account_by_uref() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::OtherSourceAccountByURef);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_missing_target() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::MissingTarget);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_missing_amount() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::MissingAmount);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_source_uref_nonexistent() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::SourceURefNonexistent);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_target_uref_nonexistent() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::TargetURefNonexistent);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_invalid_source_uref() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::SourceURefNotPurse);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_invalid_target_uref() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::TargetURefNotPurse);
}

#[ignore]
#[test]
fn should_not_transfer_wasmless_other_purse_to_self_purse() {
    invalid_transfer_wasmless(InvalidWasmlessTransfer::OtherPurseToSelfPurse);
}

enum InvalidWasmlessTransfer {
    TransferToSelfByAddr,
    TransferToSelfByKey,
    TransferToSelfByURef,
    OtherSourceAccountByAddr,
    OtherSourceAccountByKey,
    OtherSourceAccountByURef,
    MissingTarget,
    MissingAmount,
    SourceURefNotPurse,
    TargetURefNotPurse,
    SourceURefNonexistent,
    TargetURefNonexistent,
    OtherPurseToSelfPurse,
}

fn invalid_transfer_wasmless(invalid_wasmless_transfer: InvalidWasmlessTransfer) {
    let create_account_2: bool = true;
    let mut builder = init_wasmless_transform_builder(create_account_2);
    let transfer_amount: U512 = U512::from(1000);
    let id: Option<u64> = None;

    let (addr, runtime_args, expected_error) = match invalid_wasmless_transfer {
        InvalidWasmlessTransfer::TransferToSelfByAddr => {
            // same source and target purse is invalid
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_TARGET => ACCOUNT_1_ADDR,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id,
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidPurse)),
            )
        }
        InvalidWasmlessTransfer::TransferToSelfByKey => {
            // same source and target purse is invalid
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_TARGET => Key::Account(ACCOUNT_1_ADDR),
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidPurse)),
            )
        }
        InvalidWasmlessTransfer::TransferToSelfByURef => {
            let account_1_purse = builder
                .get_account(ACCOUNT_1_ADDR)
                .expect("should get account 1")
                .main_purse();
            // same source and target purse is invalid
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_TARGET => account_1_purse,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidPurse)),
            )
        }
        InvalidWasmlessTransfer::OtherSourceAccountByAddr => {
            // passes another account's addr as source
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_SOURCE => ACCOUNT_2_ADDR,
                    mint::ARG_TARGET => ACCOUNT_1_ADDR,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidArgument)),
            )
        }
        InvalidWasmlessTransfer::OtherSourceAccountByKey => {
            // passes another account's Key::Account as source
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_SOURCE => Key::Account(ACCOUNT_2_ADDR),
                    mint::ARG_TARGET => ACCOUNT_1_ADDR,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidArgument)),
            )
        }
        InvalidWasmlessTransfer::OtherSourceAccountByURef => {
            let account_2_purse = builder
                .get_account(ACCOUNT_2_ADDR)
                .expect("should get account 1")
                .main_purse();
            // passes another account's purse as source
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_SOURCE => account_2_purse,
                    mint::ARG_TARGET => ACCOUNT_1_ADDR,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::ForgedReference(account_2_purse)),
            )
        }
        InvalidWasmlessTransfer::MissingTarget => {
            // does not pass target
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::MissingArgument)),
            )
        }
        InvalidWasmlessTransfer::MissingAmount => {
            // does not pass amount
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_TARGET => ACCOUNT_2_ADDR,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::MissingArgument)),
            )
        }
        InvalidWasmlessTransfer::SourceURefNotPurse => {
            let not_purse_uref =
                get_default_account_named_uref(&mut builder, TRANSFER_RESULT_NAMED_KEY);
            // passes an invalid uref as source (an existing uref that is not a purse uref)
            (
                *DEFAULT_ACCOUNT_ADDR,
                runtime_args! {
                    mint::ARG_SOURCE => not_purse_uref,
                    mint::ARG_TARGET => ACCOUNT_1_ADDR,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidPurse)),
            )
        }
        InvalidWasmlessTransfer::TargetURefNotPurse => {
            let not_purse_uref =
                get_default_account_named_uref(&mut builder, TRANSFER_RESULT_NAMED_KEY);
            // passes an invalid uref as target (an existing uref that is not a purse uref)
            (
                *DEFAULT_ACCOUNT_ADDR,
                runtime_args! {
                    mint::ARG_TARGET => not_purse_uref,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidPurse)),
            )
        }
        InvalidWasmlessTransfer::SourceURefNonexistent => {
            let nonexistent_purse = URef::new([255; 32], AccessRights::READ_ADD_WRITE);
            // passes a nonexistent uref as source; considered to be a forged reference as when
            // a caller passes a uref as source they are claiming it is a purse and that they have
            // write access to it / are allowed to take funds from it.
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_SOURCE => nonexistent_purse,
                    mint::ARG_TARGET => ACCOUNT_1_ADDR,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::ForgedReference(nonexistent_purse)),
            )
        }
        InvalidWasmlessTransfer::TargetURefNonexistent => {
            let nonexistent_purse = URef::new([255; 32], AccessRights::READ_ADD_WRITE);
            // passes a nonexistent uref as target
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_TARGET => nonexistent_purse,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::Revert(ApiError::InvalidPurse)),
            )
        }
        InvalidWasmlessTransfer::OtherPurseToSelfPurse => {
            let account_1_purse = builder
                .get_account(ACCOUNT_1_ADDR)
                .expect("should get account 1")
                .main_purse();
            let account_2_purse = builder
                .get_account(ACCOUNT_2_ADDR)
                .expect("should get account 1")
                .main_purse();

            // attempts to take from an unowned purse
            (
                ACCOUNT_1_ADDR,
                runtime_args! {
                    mint::ARG_SOURCE => account_2_purse,
                    mint::ARG_TARGET => account_1_purse,
                    mint::ARG_AMOUNT => transfer_amount,
                    mint::ARG_ID => id
                },
                CoreError::Exec(ExecError::ForgedReference(account_2_purse)),
            )
        }
    };

    let no_wasm_transfer_request = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(addr)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[addr])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    let account_1_purse = builder
        .get_account(ACCOUNT_1_ADDR)
        .expect("should get account 1")
        .main_purse();

    let account_1_starting_balance = builder.get_purse_balance(account_1_purse);

    builder.exec(no_wasm_transfer_request);

    let result = builder
        .get_exec_results()
        .last()
        .expect("Expected to be called after run()")
        .get(0)
        .expect("Unable to get first deploy result");

    assert!(result.is_failure(), "was expected to fail");

    let error = result.as_error().expect("should have error");

    let account_1_closing_balance = builder.get_purse_balance(account_1_purse);

    assert_eq!(
        format!("{}", &expected_error),
        format!("{}", error),
        "expected_error: {} actual error: {}",
        expected_error,
        error
    );

    // No balance change expected in invalid transfer tests
    assert_eq!(account_1_starting_balance, account_1_closing_balance);

    // Make sure postconditions are met: payment purse has to be empty after finalization
    let handle_payment = builder.get_handle_payment_contract_hash();
    let contract = builder
        .get_contract(handle_payment)
        .expect("should have contract");
    let key = contract
        .named_keys()
        .get(handle_payment::PAYMENT_PURSE_KEY)
        .cloned()
        .expect("should have named key");

    assert_eq!(
        builder.get_purse_balance(key.into_uref().unwrap()),
        U512::zero()
    );
}

#[ignore]
#[test]
fn transfer_wasmless_should_create_target_if_it_doesnt_exist() {
    let wasmless_transfer_gas_cost = Gas::from(DEFAULT_WASMLESS_TRANSFER_COST);
    let wasmless_transfer_cost = Motes::from_gas(
        wasmless_transfer_gas_cost,
        WASMLESS_TRANSFER_FIXED_GAS_PRICE,
    )
    .expect("gas overflow");

    let create_account_2: bool = false;
    let mut builder = init_wasmless_transform_builder(create_account_2);
    let transfer_amount: U512 = U512::from(1000);

    let account_1_purse = builder
        .get_account(ACCOUNT_1_ADDR)
        .expect("should get account 1")
        .main_purse();

    assert_eq!(
        builder.get_account(ACCOUNT_2_ADDR),
        None,
        "account 2 should not exist"
    );

    let account_1_starting_balance = builder.get_purse_balance(account_1_purse);

    let runtime_args = runtime_args! {
       mint::ARG_TARGET => ACCOUNT_2_ADDR,
       mint::ARG_AMOUNT => transfer_amount,
       mint::ARG_ID => <Option<u64>>::None
    };

    let no_wasm_transfer_request = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_1_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_1_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder
        .exec(no_wasm_transfer_request)
        .expect_success()
        .commit();

    let account_2 = builder
        .get_account(ACCOUNT_2_ADDR)
        .expect("account 2 should exist");

    let account_2_starting_balance = builder.get_purse_balance(account_2.main_purse());

    assert_eq!(
        account_1_starting_balance - transfer_amount - wasmless_transfer_cost.value(),
        builder.get_purse_balance(account_1_purse),
        "account 1 ending balance incorrect"
    );
    assert_eq!(
        account_2_starting_balance, transfer_amount,
        "account 2 ending balance incorrect"
    );
}

fn get_default_account_named_uref(builder: &mut InMemoryWasmTestBuilder, name: &str) -> URef {
    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("default account should exist");
    default_account
        .named_keys()
        .get(name)
        .expect("default account should have named key")
        .as_uref()
        .expect("should be a uref")
        .to_owned()
}

fn init_wasmless_transform_builder(create_account_2: bool) -> InMemoryWasmTestBuilder {
    let mut builder = InMemoryWasmTestBuilder::default();

    let id: Option<u64> = None;

    let create_account_1_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNT,
        runtime_args! {
            mint::ARG_TARGET => ACCOUNT_1_ADDR,
            mint::ARG_AMOUNT => *DEFAULT_PAYMENT,
            mint::ARG_ID => id
        },
    )
    .build();

    builder
        .run_genesis(&DEFAULT_RUN_GENESIS_REQUEST)
        .exec(create_account_1_request)
        .expect_success()
        .commit();

    if !create_account_2 {
        return builder;
    }

    let create_account_2_request = ExecuteRequestBuilder::standard(
        *DEFAULT_ACCOUNT_ADDR,
        CONTRACT_TRANSFER_PURSE_TO_ACCOUNT,
        runtime_args! {
            mint::ARG_TARGET => ACCOUNT_2_ADDR,
            mint::ARG_AMOUNT => *DEFAULT_PAYMENT,
            mint::ARG_ID => id
        },
    )
    .build();

    builder
        .exec(create_account_2_request)
        .commit()
        .expect_success()
        .to_owned()
}

#[ignore]
#[test]
fn transfer_wasmless_should_fail_without_main_purse_minimum_balance() {
    let wasmless_transfer_gas_cost = Gas::from(DEFAULT_WASMLESS_TRANSFER_COST);
    let wasmless_transfer_cost = Motes::from_gas(
        wasmless_transfer_gas_cost,
        WASMLESS_TRANSFER_FIXED_GAS_PRICE,
    )
    .expect("gas overflow");

    let create_account_2: bool = false;
    let mut builder = init_wasmless_transform_builder(create_account_2);
    let account_1_to_account_2_amount: U512 =
        U512::from(DEFAULT_WASMLESS_TRANSFER_COST) - U512::one();
    let account_2_to_account_1_amount: U512 = U512::one();

    let account_1_purse = builder
        .get_account(ACCOUNT_1_ADDR)
        .expect("should get account 1")
        .main_purse();

    assert_eq!(
        builder.get_account(ACCOUNT_2_ADDR),
        None,
        "account 2 should not exist"
    );

    let account_1_starting_balance = builder.get_purse_balance(account_1_purse);

    let runtime_args = runtime_args! {
       mint::ARG_TARGET => ACCOUNT_2_ADDR,
       mint::ARG_AMOUNT => account_1_to_account_2_amount,
       mint::ARG_ID => <Option<u64>>::None
    };

    let no_wasm_transfer_request_1 = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_1_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_1_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder
        .exec(no_wasm_transfer_request_1)
        .expect_success()
        .commit();

    let account_2 = builder
        .get_account(ACCOUNT_2_ADDR)
        .expect("account 2 should exist");

    let account_2_starting_balance = builder.get_purse_balance(account_2.main_purse());

    assert_eq!(
        account_1_starting_balance - account_1_to_account_2_amount - wasmless_transfer_cost.value(),
        builder.get_purse_balance(account_1_purse),
        "account 1 ending balance incorrect"
    );
    assert_eq!(
        account_2_starting_balance, account_1_to_account_2_amount,
        "account 2 ending balance incorrect"
    );

    // Another transfer but this time created account tries to do a transfer
    assert!(account_2_to_account_1_amount < wasmless_transfer_cost.value());
    let runtime_args = runtime_args! {
       mint::ARG_TARGET => ACCOUNT_1_ADDR,
       mint::ARG_AMOUNT => account_2_to_account_1_amount,
       mint::ARG_ID => <Option<u64>>::None
    };

    let no_wasm_transfer_request_2 = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_2_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_2_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder.exec(no_wasm_transfer_request_2).commit();

    let exec_result = &builder.get_exec_results().last().unwrap()[0];
    let error = exec_result
        .as_error()
        .unwrap_or_else(|| panic!("should have error {:?}", exec_result));
    assert!(
        matches!(error, CoreError::InsufficientPayment),
        "{:?}",
        error
    );
}

#[ignore]
#[test]
fn transfer_wasmless_should_transfer_funds_after_paying_for_transfer() {
    let wasmless_transfer_gas_cost = Gas::from(DEFAULT_WASMLESS_TRANSFER_COST);
    let wasmless_transfer_cost = Motes::from_gas(
        wasmless_transfer_gas_cost,
        WASMLESS_TRANSFER_FIXED_GAS_PRICE,
    )
    .expect("gas overflow");

    let create_account_2: bool = false;
    let mut builder = init_wasmless_transform_builder(create_account_2);
    let account_1_to_account_2_amount: U512 = wasmless_transfer_cost.value() + U512::one();
    // This transfer should succeed as after paying for execution of wasmless transfer account_2's
    // main purse would contain exactly 1 token.
    let account_2_to_account_1_amount: U512 = U512::one();

    let account_1_purse = builder
        .get_account(ACCOUNT_1_ADDR)
        .expect("should get account 1")
        .main_purse();

    assert_eq!(
        builder.get_account(ACCOUNT_2_ADDR),
        None,
        "account 2 should not exist"
    );

    let account_1_starting_balance = builder.get_purse_balance(account_1_purse);

    let runtime_args = runtime_args! {
       mint::ARG_TARGET => ACCOUNT_2_ADDR,
       mint::ARG_AMOUNT => account_1_to_account_2_amount,
       mint::ARG_ID => <Option<u64>>::None
    };

    let no_wasm_transfer_request_1 = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_1_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_1_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder
        .exec(no_wasm_transfer_request_1)
        .expect_success()
        .commit();

    let account_2 = builder
        .get_account(ACCOUNT_2_ADDR)
        .expect("account 2 should exist");

    let account_2_starting_balance = builder.get_purse_balance(account_2.main_purse());

    assert_eq!(
        account_1_starting_balance - account_1_to_account_2_amount - wasmless_transfer_cost.value(),
        builder.get_purse_balance(account_1_purse),
        "account 1 ending balance incorrect"
    );
    assert_eq!(
        account_2_starting_balance, account_1_to_account_2_amount,
        "account 2 ending balance incorrect"
    );

    // Another transfer but this time created account tries to do a transfer
    assert!(account_2_to_account_1_amount <= wasmless_transfer_cost.value());
    let runtime_args = runtime_args! {
       mint::ARG_TARGET => ACCOUNT_1_ADDR,
       mint::ARG_AMOUNT => account_2_to_account_1_amount,
       mint::ARG_ID => <Option<u64>>::None
    };

    let no_wasm_transfer_request_2 = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_2_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_2_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder
        .exec(no_wasm_transfer_request_2)
        .commit()
        .expect_success();
}

#[ignore]
#[test]
fn transfer_wasmless_should_fail_with_secondary_purse_insufficient_funds() {
    let create_account_2: bool = false;
    let mut builder = init_wasmless_transform_builder(create_account_2);
    let account_1_to_account_2_amount: U512 = U512::from(1000);

    let create_purse_request = ExecuteRequestBuilder::standard(
        ACCOUNT_1_ADDR,
        CONTRACT_CREATE_PURSE_01,
        runtime_args! { ARG_PURSE_NAME => TEST_PURSE_NAME },
    )
    .build();
    builder.exec(create_purse_request).commit().expect_success();

    let account_1 = builder
        .get_account(ACCOUNT_1_ADDR)
        .expect("should get account 1");

    let account_1_purse = account_1
        .named_keys()
        .get(TEST_PURSE_NAME)
        .expect("should have purse")
        .into_uref()
        .expect("should have purse uref");

    assert_eq!(builder.get_purse_balance(account_1_purse), U512::zero());

    let account_1_starting_balance = builder.get_purse_balance(account_1_purse);
    assert_eq!(account_1_starting_balance, U512::zero());

    let runtime_args = runtime_args! {
       mint::ARG_SOURCE => account_1_purse,
       mint::ARG_TARGET => ACCOUNT_2_ADDR,
       mint::ARG_AMOUNT => account_1_to_account_2_amount,
       mint::ARG_ID => <Option<u64>>::None
    };

    let no_wasm_transfer_request_1 = {
        let deploy_item = DeployItemBuilder::new()
            .with_address(ACCOUNT_1_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(runtime_args)
            .with_authorization_keys(&[ACCOUNT_1_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item).build()
    };

    builder.exec(no_wasm_transfer_request_1).commit();

    let exec_result = &builder.get_exec_results().last().unwrap()[0];
    let error = exec_result.as_error().expect("should have error");
    assert!(
        matches!(error, CoreError::InsufficientPayment),
        "{:?}",
        error
    );
}

#[ignore]
#[test]
fn transfer_wasmless_should_observe_upgraded_cost() {
    let new_wasmless_transfer_cost_value = DEFAULT_WASMLESS_TRANSFER_COST * 2;

    let new_wasmless_transfer_gas_cost = Gas::from(new_wasmless_transfer_cost_value);
    let new_wasmless_transfer_cost = Motes::from_gas(
        new_wasmless_transfer_gas_cost,
        WASMLESS_TRANSFER_FIXED_GAS_PRICE,
    )
    .expect("gas overflow");

    let transfer_amount = U512::one();
    const DEFAULT_ACTIVATION_POINT: EraId = EraId::new(1);

    let new_auction_costs = AuctionCosts::default();
    let new_mint_costs = MintCosts::default();
    let new_handle_payment_costs = HandlePaymentCosts::default();
    let new_standard_payment_costs = StandardPaymentCosts::default();

    let new_system_costs = SystemCosts::new(
        new_wasmless_transfer_cost_value,
        new_auction_costs,
        new_mint_costs,
        new_handle_payment_costs,
        new_standard_payment_costs,
    );

    let old_protocol_version = *DEFAULT_PROTOCOL_VERSION;
    let new_protocol_version = ProtocolVersion::from_parts(
        old_protocol_version.value().major,
        old_protocol_version.value().minor,
        old_protocol_version.value().patch + 1,
    );

    let mut builder = InMemoryWasmTestBuilder::default();
    builder.run_genesis(&*DEFAULT_RUN_GENESIS_REQUEST);

    let default_account = builder
        .get_account(*DEFAULT_ACCOUNT_ADDR)
        .expect("should get default_account");

    let mut upgrade_request = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(*DEFAULT_PROTOCOL_VERSION)
            .with_new_protocol_version(new_protocol_version)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .with_new_system_costs(new_system_costs)
            .build()
    };

    builder.upgrade_with_upgrade_request(&mut upgrade_request);

    let default_account_balance_before = builder.get_purse_balance(default_account.main_purse());

    let no_wasm_transfer_request_1 = {
        let wasmless_transfer_args = runtime_args! {
        mint::ARG_TARGET => ACCOUNT_2_ADDR,
        mint::ARG_AMOUNT => transfer_amount,
        mint::ARG_ID => <Option<u64>>::None
        };

        let deploy_item = DeployItemBuilder::new()
            .with_address(*DEFAULT_ACCOUNT_ADDR)
            .with_empty_payment_bytes(runtime_args! {})
            .with_transfer_args(wasmless_transfer_args)
            .with_authorization_keys(&[*DEFAULT_ACCOUNT_ADDR])
            .build();
        ExecuteRequestBuilder::from_deploy_item(deploy_item)
            .with_protocol_version(new_protocol_version)
            .build()
    };

    builder
        .exec(no_wasm_transfer_request_1)
        .expect_success()
        .commit();

    let default_account_balance_after = builder.get_purse_balance(default_account.main_purse());

    assert_eq!(
        default_account_balance_before - transfer_amount - new_wasmless_transfer_cost.value(),
        default_account_balance_after,
        "expected wasmless transfer cost to be {} but it was {}",
        new_wasmless_transfer_cost,
        default_account_balance_before - default_account_balance_after - transfer_amount
    );
}
