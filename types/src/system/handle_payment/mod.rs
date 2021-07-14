//! Contains implementation of a Handle Payment contract functionality.
mod constants;
mod entry_points;
mod error;
mod mint_provider;
mod runtime_provider;

use core::marker::Sized;

use crate::{account::AccountHash, AccessRights, URef, U512};

pub use crate::system::handle_payment::{
    constants::*, entry_points::handle_payment_entry_points, error::Error,
    mint_provider::MintProvider, runtime_provider::RuntimeProvider,
};

// A simplified representation of a refund percentage which is currently hardcoded to 0%.
const REFUND_PERCENTAGE: U512 = U512::zero();

/// Handle payment functionality implementation.
pub trait HandlePayment: MintProvider + RuntimeProvider + Sized {
    /// Get payment purse.
    fn get_payment_purse(&self) -> Result<URef, Error> {
        let purse = internal::get_payment_purse(self)?;
        // Limit the access rights so only balance query and deposit are allowed.
        Ok(URef::new(purse.addr(), AccessRights::READ_ADD))
    }

    /// Set refund purse.
    fn set_refund_purse(&mut self, purse: URef) -> Result<(), Error> {
        internal::set_refund(self, purse)
    }

    /// Get refund purse.
    fn get_refund_purse(&self) -> Result<Option<URef>, Error> {
        // We purposely choose to remove the access rights so that we do not
        // accidentally give rights for a purse to some contract that is not
        // supposed to have it.
        let maybe_purse = internal::get_refund_purse(self)?;
        Ok(maybe_purse.map(|p| p.remove_access_rights()))
    }

    /// Finalize payment with `amount_spent` and a given `account`.
    fn finalize_payment(
        &mut self,
        amount_spent: U512,
        account: AccountHash,
        target: URef,
    ) -> Result<(), Error> {
        internal::finalize_payment(self, amount_spent, account, target)
    }
}

mod internal {
    use crate::{
        account::AccountHash,
        system::handle_payment::{Error, MintProvider, RuntimeProvider},
        Key, Phase, PublicKey, URef, U512,
    };

    use super::{PAYMENT_PURSE_KEY, REFUND_PERCENTAGE, REFUND_PURSE_KEY};

    /// Returns the purse for accepting payment for transactions.
    pub fn get_payment_purse<R: RuntimeProvider>(runtime_provider: &R) -> Result<URef, Error> {
        match runtime_provider.get_key(PAYMENT_PURSE_KEY) {
            Some(Key::URef(uref)) => Ok(uref),
            Some(_) => Err(Error::PaymentPurseKeyUnexpectedType),
            None => Err(Error::PaymentPurseNotFound),
        }
    }

    /// Sets the purse where refunds (excess funds not spent to pay for computation) will be sent.
    /// Note that if this function is never called, the default location is the main purse of the
    /// deployer's account.
    pub fn set_refund<R: RuntimeProvider>(
        runtime_provider: &mut R,
        purse: URef,
    ) -> Result<(), Error> {
        if let Phase::Payment = runtime_provider.get_phase() {
            runtime_provider.put_key(REFUND_PURSE_KEY, Key::URef(purse))?;
            return Ok(());
        }
        Err(Error::SetRefundPurseCalledOutsidePayment)
    }

    /// Returns the currently set refund purse.
    pub fn get_refund_purse<R: RuntimeProvider>(
        runtime_provider: &R,
    ) -> Result<Option<URef>, Error> {
        match runtime_provider.get_key(REFUND_PURSE_KEY) {
            Some(Key::URef(uref)) => Ok(Some(uref)),
            Some(_) => Err(Error::RefundPurseKeyUnexpectedType),
            None => Ok(None),
        }
    }

    /// Transfers funds from the payment purse to the validator rewards purse, as well as to the
    /// refund purse, depending on how much was spent on the computation. This function maintains
    /// the invariant that the balance of the payment purse is zero at the beginning and end of each
    /// deploy and that the refund purse is unset at the beginning and end of each deploy.
    pub fn finalize_payment<P: MintProvider + RuntimeProvider>(
        provider: &mut P,
        amount_spent: U512,
        account: AccountHash,
        target: URef,
    ) -> Result<(), Error> {
        let caller = provider.get_caller();
        if caller != PublicKey::System.to_account_hash() {
            return Err(Error::SystemFunctionCalledByUserAccount);
        }

        let payment_purse = get_payment_purse(provider)?;
        let total = match provider.balance(payment_purse)? {
            Some(balance) => balance,
            None => return Err(Error::PaymentPurseBalanceNotFound),
        };

        if total < amount_spent {
            return Err(Error::InsufficientPaymentForAmountSpent);
        }

        // User's part
        let refund_amount = {
            let refund_amount_raw = total
                .checked_sub(amount_spent)
                .ok_or(Error::ArithmeticOverflow)?;
            // Currently refund percentage is zero and we expect no overflows.
            // However, we put this check should the constant change in the future.
            refund_amount_raw
                .checked_mul(REFUND_PERCENTAGE)
                .ok_or(Error::ArithmeticOverflow)?
        };

        // Validator reward
        let validator_reward = total
            .checked_sub(refund_amount)
            .ok_or(Error::ArithmeticOverflow)?;

        // Makes sure both parts: for user, and for validator sums to the total amount in the
        // payment's purse.
        debug_assert_eq!(validator_reward + refund_amount, total);

        let refund_purse = get_refund_purse(provider)?;

        if let Some(refund_purse) = refund_purse {
            if refund_purse.remove_access_rights() == payment_purse.remove_access_rights() {
                // Make sure we're not refunding into a payment purse to invalidate payment code
                // postconditions.
                return Err(Error::RefundPurseIsPaymentPurse);
            }
        }

        provider.remove_key(REFUND_PURSE_KEY)?; //unset refund purse after reading it

        // pay target validator
        provider
            .transfer_purse_to_purse(payment_purse, target, validator_reward)
            .map_err(|_| Error::FailedTransferToRewardsPurse)?;

        if refund_amount.is_zero() {
            return Ok(());
        }

        // give refund
        let refund_purse = match refund_purse {
            Some(uref) => uref,
            None => return refund_to_account::<P>(provider, payment_purse, account, refund_amount),
        };

        // in case of failure to transfer to refund purse we fall back on the account's main purse
        if provider
            .transfer_purse_to_purse(payment_purse, refund_purse, refund_amount)
            .is_err()
        {
            return refund_to_account::<P>(provider, payment_purse, account, refund_amount);
        }

        Ok(())
    }

    pub fn refund_to_account<M: MintProvider>(
        mint_provider: &mut M,
        payment_purse: URef,
        account: AccountHash,
        amount: U512,
    ) -> Result<(), Error> {
        match mint_provider.transfer_purse_to_account(payment_purse, account, amount) {
            Ok(_) => Ok(()),
            Err(_) => Err(Error::FailedTransferToAccountPurse),
        }
    }
}
