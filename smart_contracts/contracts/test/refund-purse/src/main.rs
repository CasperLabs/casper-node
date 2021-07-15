#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;

use casper_contract::{
    contract_api::{account, runtime, system},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{runtime_args, ApiError, ContractHash, RuntimeArgs, URef, U512};

#[repr(u16)]
enum Error {
    ShouldNotExist = 0,
    NotFound,
    Invalid,
    IncorrectAccessRights,
}

pub const ARG_PURSE: &str = "purse";
const ARG_PAYMENT_AMOUNT: &str = "payment_amount";
const SET_REFUND_PURSE: &str = "set_refund_purse";
const GET_REFUND_PURSE: &str = "get_refund_purse";
const GET_PAYMENT_PURSE: &str = "get_payment_purse";
const ARG_PURSE_NAME_1: &str = "purse_name_1";
const ARG_PURSE_NAME_2: &str = "purse_name_2";

fn set_refund_purse(contract_hash: ContractHash, p: &URef) {
    runtime::call_contract(
        contract_hash,
        SET_REFUND_PURSE,
        runtime_args! {
            ARG_PURSE => *p,
        },
    )
}

fn get_refund_purse(handle_payment: ContractHash) -> Option<URef> {
    runtime::call_contract(handle_payment, GET_REFUND_PURSE, runtime_args! {})
}

fn get_payment_purse(handle_payment: ContractHash) -> URef {
    runtime::call_contract(handle_payment, GET_PAYMENT_PURSE, runtime_args! {})
}

fn submit_payment(handle_payment: ContractHash, amount: U512) {
    let payment_purse = get_payment_purse(handle_payment);
    let main_purse = account::get_main_purse();
    system::transfer_from_purse_to_purse(main_purse, payment_purse, amount, None).unwrap_or_revert()
}

#[no_mangle]
pub extern "C" fn call() {
    let handle_payment = system::get_handle_payment();

    let refund_purse_name_1: String = runtime::get_named_arg(ARG_PURSE_NAME_1);
    let refund_purse_name_2: String = runtime::get_named_arg(ARG_PURSE_NAME_2);

    let refund_purse_1 = runtime::get_key(&refund_purse_name_1)
        .unwrap_or_revert()
        .into_uref()
        .unwrap_or_revert();
    {
        // get_refund_purse should return None before setting it
        let refund_result = get_refund_purse(handle_payment);
        if refund_result.is_some() {
            runtime::revert(ApiError::User(Error::ShouldNotExist as u16));
        }

        // it should return Some(x) after calling set_refund_purse(x)
        set_refund_purse(handle_payment, &refund_purse_1);
        let refund_purse = match get_refund_purse(handle_payment) {
            None => runtime::revert(ApiError::User(Error::NotFound as u16)),
            Some(x) if x.addr() == refund_purse_1.addr() => x,
            Some(_) => runtime::revert(ApiError::User(Error::Invalid as u16)),
        };

        // the returned purse should not have any access rights
        if refund_purse.is_addable() || refund_purse.is_writeable() || refund_purse.is_readable() {
            runtime::revert(ApiError::User(Error::IncorrectAccessRights as u16))
        }
    }
    {
        let refund_purse_2 = runtime::get_key(&refund_purse_name_2)
            .unwrap_or_revert()
            .into_uref()
            .unwrap_or_revert();
        // get_refund_purse should return correct value after setting a second time
        set_refund_purse(handle_payment, &refund_purse_2);
        match get_refund_purse(handle_payment) {
            None => runtime::revert(ApiError::User(Error::NotFound as u16)),
            Some(uref) if uref.addr() == refund_purse_2.addr() => (),
            Some(_) => runtime::revert(ApiError::User(Error::Invalid as u16)),
        }

        let payment_amount: U512 = runtime::get_named_arg(ARG_PAYMENT_AMOUNT);
        submit_payment(handle_payment, payment_amount);
    }
}
