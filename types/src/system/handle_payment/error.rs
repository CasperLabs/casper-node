//! Home of the Handle Payment contract's [`Error`] type.
use alloc::vec::Vec;
use core::result;

#[cfg(feature = "std")]
use thiserror::Error;

use crate::{
    bytesrepr::{self, ToBytes, U8_SERIALIZED_LENGTH},
    CLType, CLTyped,
};

/// Errors which can occur while executing the Handle Payment contract.
// TODO: Split this up into user errors vs. system errors.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(Error))]
#[repr(u8)]
pub enum Error {
    // ===== User errors =====
    /// The given validator is not bonded.
    #[cfg_attr(feature = "std", error("Not bonded"))]
    NotBonded = 0,
    /// There are too many bonding or unbonding attempts already enqueued to allow more.
    #[cfg_attr(feature = "std", error("Too many events in queue"))]
    TooManyEventsInQueue,
    /// At least one validator must remain bonded.
    #[cfg_attr(feature = "std", error("Cannot unbond last validator"))]
    CannotUnbondLastValidator,
    /// Failed to bond or unbond as this would have resulted in exceeding the maximum allowed
    /// difference between the largest and smallest stakes.
    #[cfg_attr(feature = "std", error("Spread is too high"))]
    SpreadTooHigh,
    /// The given validator already has a bond or unbond attempt enqueued.
    #[cfg_attr(feature = "std", error("Multiple requests"))]
    MultipleRequests,
    /// Attempted to bond with a stake which was too small.
    #[cfg_attr(feature = "std", error("Bond is too small"))]
    BondTooSmall,
    /// Attempted to bond with a stake which was too large.
    #[cfg_attr(feature = "std", error("Bond is too large"))]
    BondTooLarge,
    /// Attempted to unbond an amount which was too large.
    #[cfg_attr(feature = "std", error("Unbond is too large"))]
    UnbondTooLarge,
    /// While bonding, the transfer from source purse to the Handle Payment internal purse failed.
    #[cfg_attr(feature = "std", error("Bond transfer failed"))]
    BondTransferFailed,
    /// While unbonding, the transfer from the Handle Payment internal purse to the destination
    /// purse failed.
    #[cfg_attr(feature = "std", error("Unbond transfer failed"))]
    UnbondTransferFailed,
    // ===== System errors =====
    /// Internal error: a [`BlockTime`](crate::BlockTime) was unexpectedly out of sequence.
    #[cfg_attr(feature = "std", error("Time went backwards"))]
    TimeWentBackwards,
    /// Internal error: stakes were unexpectedly empty.
    #[cfg_attr(feature = "std", error("Stakes not found"))]
    StakesNotFound,
    /// Internal error: the Handle Payment contract's payment purse wasn't found.
    #[cfg_attr(feature = "std", error("Payment purse not found"))]
    PaymentPurseNotFound,
    /// Internal error: the Handle Payment contract's payment purse key was the wrong type.
    #[cfg_attr(feature = "std", error("Payment purse has unexpected type"))]
    PaymentPurseKeyUnexpectedType,
    /// Internal error: couldn't retrieve the balance for the Handle Payment contract's payment
    /// purse.
    #[cfg_attr(feature = "std", error("Payment purse balance not found"))]
    PaymentPurseBalanceNotFound,
    /// Internal error: the Handle Payment contract's bonding purse wasn't found.
    #[cfg_attr(feature = "std", error("Bonding purse not found"))]
    BondingPurseNotFound,
    /// Internal error: the Handle Payment contract's bonding purse key was the wrong type.
    #[cfg_attr(feature = "std", error("Bonding purse key has unexpected type"))]
    BondingPurseKeyUnexpectedType,
    /// Internal error: the Handle Payment contract's refund purse key was the wrong type.
    #[cfg_attr(feature = "std", error("Refund purse key has unexpected type"))]
    RefundPurseKeyUnexpectedType,
    /// Internal error: the Handle Payment contract's rewards purse wasn't found.
    #[cfg_attr(feature = "std", error("Rewards purse not found"))]
    RewardsPurseNotFound,
    /// Internal error: the Handle Payment contract's rewards purse key was the wrong type.
    #[cfg_attr(feature = "std", error("Rewards purse has unexpected type"))]
    RewardsPurseKeyUnexpectedType,
    // TODO: Put these in their own enum, and wrap them separately in `BondingError` and
    //       `UnbondingError`.
    /// Internal error: failed to deserialize the stake's key.
    #[cfg_attr(feature = "std", error("Failed to deserialize stake's key"))]
    StakesKeyDeserializationFailed,
    /// Internal error: failed to deserialize the stake's balance.
    #[cfg_attr(feature = "std", error("Failed to deserialize stake's balance"))]
    StakesDeserializationFailed,
    /// The invoked Handle Payment function can only be called by system contracts, but was called
    /// by a user contract.
    #[cfg_attr(feature = "std", error("System function was called by user account"))]
    SystemFunctionCalledByUserAccount,
    /// Internal error: while finalizing payment, the amount spent exceeded the amount available.
    #[cfg_attr(feature = "std", error("Insufficient payment for amount spent"))]
    InsufficientPaymentForAmountSpent,
    /// Internal error: while finalizing payment, failed to pay the validators (the transfer from
    /// the Handle Payment contract's payment purse to rewards purse failed).
    #[cfg_attr(feature = "std", error("Transfer to rewards purse has failed"))]
    FailedTransferToRewardsPurse,
    /// Internal error: while finalizing payment, failed to refund the caller's purse (the transfer
    /// from the Handle Payment contract's payment purse to refund purse or account's main purse
    /// failed).
    #[cfg_attr(feature = "std", error("Transfer to account's purse failed"))]
    FailedTransferToAccountPurse,
    /// Handle Payment contract's "set_refund_purse" method can only be called by the payment code
    /// of a deploy, but was called by the session code.
    #[cfg_attr(feature = "std", error("Set refund purse was called outside payment"))]
    SetRefundPurseCalledOutsidePayment,
    /// Raised when the system is unable to determine purse balance.
    #[cfg_attr(feature = "std", error("Unable to get purse balance"))]
    GetBalance,
    /// Raised when the system is unable to put named key.
    #[cfg_attr(feature = "std", error("Unable to put named key"))]
    PutKey,
    /// Raised when the system is unable to remove given named key.
    #[cfg_attr(feature = "std", error("Unable to remove named key"))]
    RemoveKey,
    /// Failed to transfer funds.
    #[cfg_attr(feature = "std", error("Failed to transfer funds"))]
    Transfer,
    /// An arithmetic overflow occurred
    #[cfg_attr(feature = "std", error("Arithmetic overflow"))]
    ArithmeticOverflow,
    // NOTE: These variants below will be removed once support for WASM system contracts will be
    // dropped.
    #[doc(hidden)]
    #[cfg_attr(feature = "std", error("GasLimit"))]
    GasLimit,
    /// Refund purse is a payment purse.
    #[cfg_attr(feature = "std", error("Refund purse is a payment purse."))]
    RefundPurseIsPaymentPurse,
}

impl CLTyped for Error {
    fn cl_type() -> CLType {
        CLType::U8
    }
}

impl ToBytes for Error {
    fn to_bytes(&self) -> result::Result<Vec<u8>, bytesrepr::Error> {
        let value = *self as u8;
        value.to_bytes()
    }

    fn serialized_length(&self) -> usize {
        U8_SERIALIZED_LENGTH
    }
}
