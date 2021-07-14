#![no_std]
#![no_main]

use casper_contract::{
    contract_api::{account, runtime, system},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{
    runtime_args, system::handle_payment, ApiError, Phase, RuntimeArgs, URef, U512,
};

const ARG_AMOUNT: &str = "amount";

#[repr(u16)]
enum Error {
    InvalidPhase,
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError::User(e as u16)
    }
}

fn get_payment_purse() -> URef {
    runtime::call_contract(
        system::get_handle_payment(),
        handle_payment::METHOD_GET_PAYMENT_PURSE,
        RuntimeArgs::default(),
    )
}

fn set_refund_purse(new_refund_purse: URef) {
    let args = runtime_args! {
        handle_payment::ARG_PURSE => new_refund_purse,
    };

    runtime::call_contract(
        system::get_handle_payment(),
        handle_payment::METHOD_SET_REFUND_PURSE,
        args,
    )
}

#[no_mangle]
pub extern "C" fn call() {
    if runtime::get_phase() != Phase::Payment {
        runtime::revert(Error::InvalidPhase);
    }

    let amount: U512 = runtime::get_named_arg(ARG_AMOUNT);

    // Attempt to get refund into a payment purse.
    let payment_purse = get_payment_purse();
    set_refund_purse(payment_purse);

    // transfer amount from named purse to payment purse, which will be used to pay for execution
    system::transfer_from_purse_to_purse(account::get_main_purse(), payment_purse, amount, None)
        .unwrap_or_revert();
}
