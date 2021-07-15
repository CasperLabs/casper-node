#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;

use casper_contract::{
    contract_api::{account, runtime, system},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{account::AccountHash, runtime_args, ContractHash, RuntimeArgs, URef, U512};

pub const ARG_AMOUNT: &str = "amount";
pub const ARG_AMOUNT_SPENT: &str = "amount_spent";
pub const ARG_REFUND_FLAG: &str = "refund";
pub const ARG_PURSE: &str = "purse";
pub const ARG_ACCOUNT_KEY: &str = "account";
pub const ARG_PURSE_NAME: &str = "purse_name";

fn set_refund_purse(contract_hash: ContractHash, purse: URef) {
    runtime::call_contract(
        contract_hash,
        "set_refund_purse",
        runtime_args! {
            ARG_PURSE => purse,
        },
    )
}

fn get_payment_purse(contract_hash: ContractHash) -> URef {
    runtime::call_contract(contract_hash, "get_payment_purse", runtime_args! {})
}

fn submit_payment(contract_hash: ContractHash, amount: U512) {
    let payment_purse = get_payment_purse(contract_hash);
    let main_purse = account::get_main_purse();
    system::transfer_from_purse_to_purse(main_purse, payment_purse, amount, None).unwrap_or_revert()
}

fn finalize_payment(contract_hash: ContractHash, amount_spent: U512, account: AccountHash) {
    runtime::call_contract(
        contract_hash,
        "finalize_payment",
        runtime_args! {
            ARG_AMOUNT => amount_spent,
            ARG_ACCOUNT_KEY => account,
        },
    )
}

#[no_mangle]
pub extern "C" fn call() {
    let contract_hash = system::get_handle_payment();

    let payment_amount: U512 = runtime::get_named_arg(ARG_AMOUNT);
    let refund_purse_flag: u8 = runtime::get_named_arg(ARG_REFUND_FLAG);
    let maybe_amount_spent: Option<U512> = runtime::get_named_arg(ARG_AMOUNT_SPENT);
    let maybe_account: Option<AccountHash> = runtime::get_named_arg(ARG_ACCOUNT_KEY);
    let purse_name: String = runtime::get_named_arg(ARG_PURSE_NAME);

    submit_payment(contract_hash, payment_amount);

    if refund_purse_flag != 0 {
        let refund_purse = {
            let stored_purse_key = runtime::get_key(&purse_name).unwrap_or_revert();
            stored_purse_key.into_uref().unwrap_or_revert()
        };
        set_refund_purse(contract_hash, refund_purse);
    }

    if let (Some(amount_spent), Some(account)) = (maybe_amount_spent, maybe_account) {
        finalize_payment(contract_hash, amount_spent, account);
    }
}
