//! Contains implementation of a Auction contract functionality.
mod bid;
mod constants;
mod delegator;
mod detail;
mod entry_points;
mod era_info;
mod error;
mod providers;
mod seigniorage_recipient;
mod unbonding_purse;

use alloc::{collections::BTreeMap, vec::Vec};

use num_rational::Ratio;
use num_traits::{CheckedMul, CheckedSub};

use crate::{account::AccountHash, system::CallStackElement, EraId, PublicKey, U512};

pub use bid::Bid;
pub use constants::*;
pub use delegator::Delegator;
pub use entry_points::auction_entry_points;
pub use era_info::*;
pub use error::Error;
pub use providers::{
    AccountProvider, MintProvider, RuntimeProvider, StorageProvider, SystemProvider,
};
pub use seigniorage_recipient::SeigniorageRecipient;
pub use unbonding_purse::UnbondingPurse;

/// Representation of delegation rate of tokens. Range from 0..=100.
pub type DelegationRate = u8;

/// Validators mapped to their bids.
pub type Bids = BTreeMap<PublicKey, Bid>;

/// Weights of validators. "Weight" in this context means a sum of their stakes.
pub type ValidatorWeights = BTreeMap<PublicKey, U512>;

/// List of era validators
pub type EraValidators = BTreeMap<EraId, ValidatorWeights>;

/// Collection of seigniorage recipients.
pub type SeigniorageRecipients = BTreeMap<PublicKey, SeigniorageRecipient>;

/// Snapshot of `SeigniorageRecipients` for a given era.
pub type SeigniorageRecipientsSnapshot = BTreeMap<EraId, SeigniorageRecipients>;

/// Validators and delegators mapped to their unbonding purses.
pub type UnbondingPurses = BTreeMap<AccountHash, Vec<UnbondingPurse>>;

/// Bonding auction contract interface
pub trait Auction:
    StorageProvider + SystemProvider + RuntimeProvider + MintProvider + AccountProvider + Sized
{
    /// Returns active validators and auction winners for a number of future eras determined by the
    /// configured auction_delay.
    fn get_era_validators(&mut self) -> Result<EraValidators, Error> {
        let snapshot = detail::get_seigniorage_recipients_snapshot(self)?;
        let era_validators = snapshot
            .into_iter()
            .map(|(era_id, recipients)| {
                let validator_weights = recipients
                    .into_iter()
                    .filter_map(|(public_key, bid)| {
                        bid.total_stake().map(|stake| (public_key, stake))
                    })
                    .collect::<ValidatorWeights>();
                (era_id, validator_weights)
            })
            .collect::<BTreeMap<EraId, ValidatorWeights>>();
        Ok(era_validators)
    }

    /// Returns validators in era_validators, mapped to their bids or founding stakes, delegation
    /// rates and lists of delegators together with their delegated quantities from delegators.
    /// This function is publicly accessible, but intended for system use by the Handle Payment
    /// contract, because this data is necessary for distributing seigniorage.
    fn read_seigniorage_recipients(&mut self) -> Result<SeigniorageRecipients, Error> {
        // `era_validators` are assumed to be computed already by calling "run_auction" entrypoint.
        let era_index = detail::get_era_id(self)?;
        let mut seigniorage_recipients_snapshot =
            detail::get_seigniorage_recipients_snapshot(self)?;
        let seigniorage_recipients = seigniorage_recipients_snapshot
            .remove(&era_index)
            .ok_or(Error::MissingSeigniorageRecipients)?;
        Ok(seigniorage_recipients)
    }

    /// This entry point adds or modifies an entry in the `Key::Bid` section of the global state and
    /// creates (or tops off) a bid purse. Post genesis, any new call on this entry point causes a
    /// non-founding validator in the system to exist.
    ///
    /// The logic works for both founding and non-founding validators, making it possible to adjust
    /// their delegation rate and increase their stakes.
    ///
    /// A validator with its bid inactive due to slashing can activate his bid again by increasing
    /// its stake.
    ///
    /// Validators cannot create a bid with 0 amount, and the delegation rate can't exceed
    /// [`DELEGATION_RATE_DENOMINATOR`].
    fn add_bid(
        &mut self,
        public_key: PublicKey,
        delegation_rate: DelegationRate,
        amount: U512,
    ) -> Result<U512, Error> {
        let provided_account_hash = AccountHash::from_public_key(&public_key, |x| self.blake2b(x));
        match self.get_immediate_caller() {
            Some(&CallStackElement::Session { account_hash })
                if account_hash != provided_account_hash =>
            {
                return Err(Error::InvalidContext)
            }
            Some(&CallStackElement::StoredSession { .. }) => {
                // stored session code is not allowed to call this method
                return Err(Error::InvalidContext);
            }
            _ => {}
        };

        if amount.is_zero() {
            return Err(Error::BondTooSmall);
        }

        if delegation_rate > DELEGATION_RATE_DENOMINATOR {
            return Err(Error::DelegationRateTooLarge);
        }

        let source = self.get_main_purse()?;

        let account_hash = AccountHash::from(&public_key);

        // Update bids or stakes
        let updated_amount = match self.read_bid(&account_hash)? {
            Some(mut bid) => {
                if bid.inactive() {
                    bid.activate();
                }
                self.transfer_purse_to_purse(source, *bid.bonding_purse(), amount)
                    .map_err(|_| Error::TransferToBidPurse)?;
                let updated_amount = bid
                    .with_delegation_rate(delegation_rate)
                    .increase_stake(amount)?;
                self.write_bid(account_hash, bid)?;
                updated_amount
            }
            None => {
                let bonding_purse = self.create_purse()?;
                self.transfer_purse_to_purse(source, bonding_purse, amount)
                    .map_err(|_| Error::TransferToBidPurse)?;
                let bid = Bid::unlocked(public_key, bonding_purse, amount, delegation_rate);
                self.write_bid(account_hash, bid)?;
                amount
            }
        };

        Ok(updated_amount)
    }

    /// For a non-founder validator, this entry point implements essentially the same logic as
    /// `add_bid` but reducing the number of tokens staked in the bid.
    ///
    /// For a founding validator, this function first checks whether they are released and fails if
    /// they are not.
    ///
    /// All the delegators that bid their tokens on a particular validator are automatically
    /// unbonded with their total staked amount.
    ///
    /// The function returns the new amount of motes remaining in the bid. If the target bid does
    /// not exist, the function call returns an error.
    fn withdraw_bid(&mut self, public_key: PublicKey, amount: U512) -> Result<U512, Error> {
        let provided_account_hash = AccountHash::from_public_key(&public_key, |x| self.blake2b(x));
        match self.get_immediate_caller() {
            Some(&CallStackElement::Session { account_hash })
                if account_hash != provided_account_hash =>
            {
                return Err(Error::InvalidContext)
            }
            Some(&CallStackElement::StoredSession { .. }) => {
                // stored session code is not allowed to call this method
                return Err(Error::InvalidContext);
            }
            _ => {}
        };

        let mut bid = self
            .read_bid(&provided_account_hash)?
            .ok_or(Error::ValidatorNotFound)?;

        let era_end_timestamp_millis = detail::get_era_end_timestamp_millis(self)?;

        // Fails if requested amount is greater than either the total stake or the amount of vested
        // stake.
        let updated_stake = bid.decrease_stake(amount, era_end_timestamp_millis)?;

        detail::create_unbonding_purse(
            self,
            public_key.clone(),
            public_key.clone(), // validator is the unbonder
            *bid.bonding_purse(),
            amount,
        )?;

        if updated_stake.is_zero() {
            // Automatically unbond delegators
            for (delegator_public_key, delegator) in bid.delegators() {
                detail::create_unbonding_purse(
                    self,
                    public_key.clone(),
                    delegator_public_key.clone(),
                    *delegator.bonding_purse(),
                    *delegator.staked_amount(),
                )?;
            }

            *bid.delegators_mut() = BTreeMap::new();

            bid.deactivate();
        }

        self.write_bid(provided_account_hash, bid)?;

        Ok(updated_stake)
    }

    /// Adds a new delegator to delegators or tops its current stake. If the target validator is
    /// missing, the function call returns an error and does nothing.
    ///
    /// The function transfers motes from the source purse to the delegator's bonding purse.
    ///    
    /// This entry point returns the number of tokens currently delegated to a given validator.
    fn delegate(
        &mut self,
        delegator_public_key: PublicKey,
        validator_public_key: PublicKey,
        amount: U512,
    ) -> Result<U512, Error> {
        let provided_account_hash =
            AccountHash::from_public_key(&delegator_public_key, |x| self.blake2b(x));
        match self.get_immediate_caller() {
            Some(&CallStackElement::Session { account_hash })
                if account_hash != provided_account_hash =>
            {
                return Err(Error::InvalidContext)
            }
            Some(&CallStackElement::StoredSession { .. }) => {
                // stored session code is not allowed to call this method
                return Err(Error::InvalidContext);
            }
            _ => {}
        };

        if amount.is_zero() {
            return Err(Error::BondTooSmall);
        }

        let source = self.get_main_purse()?;

        let validator_account_hash = AccountHash::from(&validator_public_key);

        let mut bid = match self.read_bid(&validator_account_hash)? {
            Some(bid) => bid,
            None => {
                // Return early if target validator is not in `bids`
                return Err(Error::ValidatorNotFound);
            }
        };

        let delegators = bid.delegators_mut();

        let new_delegation_amount = match delegators.get_mut(&delegator_public_key) {
            Some(delegator) => {
                self.transfer_purse_to_purse(source, *delegator.bonding_purse(), amount)
                    .map_err(|_| Error::TransferToDelegatorPurse)?;
                delegator.increase_stake(amount)?;
                *delegator.staked_amount()
            }
            None => {
                let bonding_purse = self.create_purse()?;
                self.transfer_purse_to_purse(source, bonding_purse, amount)
                    .map_err(|_| Error::TransferToDelegatorPurse)?;
                let delegator = Delegator::unlocked(
                    delegator_public_key.clone(),
                    amount,
                    bonding_purse,
                    validator_public_key,
                );
                delegators.insert(delegator_public_key.clone(), delegator);
                amount
            }
        };

        self.write_bid(validator_account_hash, bid)?;

        Ok(new_delegation_amount)
    }

    /// Removes specified amount of motes (or the value from the collection altogether, if the
    /// remaining amount is 0) from the entry in delegators map for given validator and creates a
    /// new unbonding request to the queue.
    ///
    /// The arguments are the delegator's key, the validator's key, and the remaining stake and
    /// return the remaining bid amount.
    fn undelegate(
        &mut self,
        delegator_public_key: PublicKey,
        validator_public_key: PublicKey,
        amount: U512,
    ) -> Result<U512, Error> {
        let provided_account_hash =
            AccountHash::from_public_key(&delegator_public_key, |x| self.blake2b(x));
        match self.get_immediate_caller() {
            Some(&CallStackElement::Session { account_hash })
                if account_hash != provided_account_hash =>
            {
                return Err(Error::InvalidContext)
            }
            Some(&CallStackElement::StoredSession { .. }) => {
                // stored session code is not allowed to call this method
                return Err(Error::InvalidContext);
            }
            _ => {}
        };

        let validator_account_hash = AccountHash::from(&validator_public_key);
        let mut bid = match self.read_bid(&validator_account_hash)? {
            Some(bid) => bid,
            None => return Err(Error::ValidatorNotFound),
        };

        let delegators = bid.delegators_mut();

        let new_amount = match delegators.get_mut(&delegator_public_key) {
            Some(delegator) => {
                detail::create_unbonding_purse(
                    self,
                    validator_public_key,
                    delegator_public_key.clone(),
                    *delegator.bonding_purse(),
                    amount,
                )?;

                let era_end_timestamp_millis = detail::get_era_end_timestamp_millis(self)?;
                let updated_stake = delegator.decrease_stake(amount, era_end_timestamp_millis)?;
                if updated_stake == U512::zero() {
                    delegators.remove(&delegator_public_key);
                };
                updated_stake
            }
            None => return Err(Error::DelegatorNotFound),
        };

        self.write_bid(validator_account_hash, bid)?;

        Ok(new_amount)
    }

    /// Slashes each validator.
    ///
    /// This can be only invoked through a system call.
    fn slash(&mut self, validator_public_keys: Vec<PublicKey>) -> Result<(), Error> {
        if self.get_caller() != PublicKey::System.to_account_hash() {
            return Err(Error::InvalidCaller);
        }

        let mut burned_amount: U512 = U512::zero();

        for validator_public_key in validator_public_keys {
            // Burn stake, deactivate
            let validator_account_hash = AccountHash::from(&validator_public_key);
            if let Some(mut bid) = self.read_bid(&validator_account_hash)? {
                burned_amount += *bid.staked_amount();
                *bid.staked_amount_mut() = U512::zero();
                bid.deactivate();
                // Reset delegator stakes when deactivating validator bid.
                for delegator in bid.delegators_mut().values_mut() {
                    *delegator.staked_amount_mut() = U512::zero();
                }
                self.write_bid(validator_account_hash, bid)?;
            };

            let validator_account_hash = AccountHash::from(&validator_public_key);
            // Update unbonding entries for given validator
            let unbonding_purses = self.read_withdraw(&validator_account_hash)?;
            if !unbonding_purses.is_empty() {
                burned_amount += unbonding_purses
                    .into_iter()
                    .map(|unbonding_purse| *unbonding_purse.amount())
                    .sum();
                self.write_withdraw(validator_account_hash, Vec::new())?;
            }
        }

        self.reduce_total_supply(burned_amount)?;

        Ok(())
    }

    /// Takes active_bids and delegators to construct a list of validators' total bids (their own
    /// added to their delegators') ordered by size from largest to smallest, then takes the top N
    /// (number of auction slots) bidders and replaces era_validators with these.
    ///
    /// Accessed by: node
    fn run_auction(
        &mut self,
        era_end_timestamp_millis: u64,
        evicted_validators: Vec<PublicKey>,
    ) -> Result<(), Error> {
        if self.get_caller() != PublicKey::System.to_account_hash() {
            return Err(Error::InvalidCaller);
        }

        let validator_slots = detail::get_validator_slots(self)?;
        let auction_delay = detail::get_auction_delay(self)?;
        let snapshot_size = auction_delay as usize + 1;
        let mut era_id: EraId = detail::get_era_id(self)?;
        let mut bids = detail::get_bids(self)?;

        // Process unbond requests
        detail::process_unbond_requests(self)?;

        // Process bids
        let mut bids_modified = false;
        for (validator_public_key, bid) in bids.iter_mut() {
            if bid.process(era_end_timestamp_millis) {
                bids_modified = true;
            }

            if evicted_validators.contains(validator_public_key) {
                bids_modified = bid.deactivate()
            }
        }

        // Compute next auction winners
        let winners: ValidatorWeights = {
            let founder_weights: ValidatorWeights = bids
                .iter()
                .filter(|(_public_key, bid)| bid.vesting_schedule().is_some() && !bid.inactive())
                .map(|(public_key, bid)| {
                    let total_staked_amount = bid.total_staked_amount()?;
                    Ok((public_key.clone(), total_staked_amount))
                })
                .collect::<Result<ValidatorWeights, Error>>()?;

            // We collect these into a vec for sorting
            let mut non_founder_weights: Vec<(PublicKey, U512)> = bids
                .iter()
                .filter(|(_public_key, bid)| bid.vesting_schedule().is_none() && !bid.inactive())
                .map(|(public_key, bid)| {
                    let total_staked_amount = bid.total_staked_amount()?;
                    Ok((public_key.clone(), total_staked_amount))
                })
                .collect::<Result<Vec<(PublicKey, U512)>, Error>>()?;

            non_founder_weights.sort_by(|(_, lhs), (_, rhs)| rhs.cmp(lhs));

            let remaining_auction_slots = validator_slots.saturating_sub(founder_weights.len());

            founder_weights
                .into_iter()
                .chain(
                    non_founder_weights
                        .into_iter()
                        .take(remaining_auction_slots),
                )
                .collect()
        };

        // Increment era
        era_id = era_id.checked_add(1).ok_or(Error::ArithmeticOverflow)?;

        let delayed_era = era_id
            .checked_add(auction_delay)
            .ok_or(Error::ArithmeticOverflow)?;

        // Update seigniorage recipients for current era
        {
            let mut snapshot = detail::get_seigniorage_recipients_snapshot(self)?;

            let mut recipients = SeigniorageRecipients::new();

            for era_validator in winners.keys() {
                let seigniorage_recipient = match bids.get(era_validator) {
                    Some(bid) => bid.into(),
                    None => return Err(Error::BidNotFound),
                };
                recipients.insert(era_validator.clone(), seigniorage_recipient);
            }

            let previous_recipients = snapshot.insert(delayed_era, recipients);
            assert!(previous_recipients.is_none());

            let snapshot = snapshot.into_iter().rev().take(snapshot_size).collect();
            detail::set_seigniorage_recipients_snapshot(self, snapshot)?;
        }

        detail::set_era_id(self, era_id)?;
        detail::set_era_end_timestamp_millis(self, era_end_timestamp_millis)?;

        if bids_modified {
            detail::set_bids(self, bids)?;
        }

        Ok(())
    }

    /// Mint and distribute seigniorage rewards to validators and their delegators,
    /// according to `reward_factors` returned by the consensus component.
    fn distribute(&mut self, reward_factors: BTreeMap<PublicKey, u64>) -> Result<(), Error> {
        if self.get_caller() != PublicKey::System.to_account_hash() {
            return Err(Error::InvalidCaller);
        }

        let seigniorage_recipients = self.read_seigniorage_recipients()?;
        let base_round_reward = self.read_base_round_reward()?;
        let era_id = detail::get_era_id(self)?;

        if reward_factors.keys().ne(seigniorage_recipients.keys()) {
            return Err(Error::MismatchedEraValidators);
        }

        let mut era_info = EraInfo::new();
        let mut seigniorage_allocations = era_info.seigniorage_allocations_mut();

        for (public_key, reward_factor) in reward_factors {
            let recipient = seigniorage_recipients
                .get(&public_key)
                .ok_or(Error::ValidatorNotFound)?;

            let total_stake = recipient.total_stake().ok_or(Error::ArithmeticOverflow)?;
            if total_stake.is_zero() {
                // TODO: error?
                continue;
            }

            let total_reward: Ratio<U512> = {
                let reward_rate = Ratio::new(U512::from(reward_factor), U512::from(BLOCK_REWARD));
                reward_rate
                    .checked_mul(&Ratio::from(base_round_reward))
                    .ok_or(Error::ArithmeticOverflow)?
            };

            let delegator_total_stake: U512 = recipient
                .delegator_total_stake()
                .ok_or(Error::ArithmeticOverflow)?;

            let delegators_part: Ratio<U512> = {
                let commission_rate = Ratio::new(
                    U512::from(*recipient.delegation_rate()),
                    U512::from(DELEGATION_RATE_DENOMINATOR),
                );
                let reward_multiplier: Ratio<U512> = Ratio::new(delegator_total_stake, total_stake);
                let delegator_reward: Ratio<U512> = total_reward
                    .checked_mul(&reward_multiplier)
                    .ok_or(Error::ArithmeticOverflow)?;
                let commission: Ratio<U512> = delegator_reward
                    .checked_mul(&commission_rate)
                    .ok_or(Error::ArithmeticOverflow)?;
                delegator_reward
                    .checked_sub(&commission)
                    .ok_or(Error::ArithmeticOverflow)?
            };

            let delegator_rewards =
                recipient
                    .delegator_stake()
                    .iter()
                    .map(|(delegator_key, delegator_stake)| {
                        let reward_multiplier = Ratio::new(*delegator_stake, delegator_total_stake);
                        let reward = delegators_part * reward_multiplier;
                        (delegator_key.clone(), reward)
                    });
            let delegator_payouts = detail::reinvest_delegator_rewards(
                self,
                &mut seigniorage_allocations,
                public_key.clone(),
                delegator_rewards,
            )?;
            let total_delegator_payout = delegator_payouts
                .iter()
                .map(|(amount, _bonding_purse)| *amount)
                .sum();

            let validators_part: Ratio<U512> = total_reward - Ratio::from(total_delegator_payout);
            let validator_reward = validators_part.to_integer();
            let validator_bonding_purse = detail::reinvest_validator_reward(
                self,
                &mut seigniorage_allocations,
                public_key.clone(),
                validator_reward,
            )?;
            // TODO: add "mint into existing purse" facility
            let tmp_validator_reward_purse =
                self.mint(validator_reward).map_err(|_| Error::MintReward)?;
            self.transfer_purse_to_purse(
                tmp_validator_reward_purse,
                validator_bonding_purse,
                validator_reward,
            )
            .map_err(|_| Error::ValidatorRewardTransfer)?;

            // TODO: add "mint into existing purse" facility
            let tmp_delegator_reward_purse = self
                .mint(total_delegator_payout)
                .map_err(|_| Error::MintReward)?;
            for (delegator_payout, bonding_purse) in delegator_payouts {
                self.transfer_purse_to_purse(
                    tmp_delegator_reward_purse,
                    bonding_purse,
                    delegator_payout,
                )
                .map_err(|_| Error::DelegatorRewardTransfer)?;
            }
        }

        self.record_era_info(era_id, era_info)?;

        Ok(())
    }

    /// Reads current era id.
    fn read_era_id(&mut self) -> Result<EraId, Error> {
        detail::get_era_id(self)
    }

    /// Activates a given validator's bid.  To be used when a validator has been marked as inactive
    /// by consensus (aka "evicted").
    fn activate_bid(&mut self, validator_public_key: PublicKey) -> Result<(), Error> {
        let provided_account_hash =
            AccountHash::from_public_key(&validator_public_key, |x| self.blake2b(x));
        match self.get_immediate_caller() {
            Some(&CallStackElement::Session { account_hash })
                if account_hash != provided_account_hash =>
            {
                return Err(Error::InvalidContext)
            }
            Some(&CallStackElement::StoredSession { .. }) => {
                // stored session code is not allowed to call this method
                return Err(Error::InvalidContext);
            }
            _ => {}
        };

        let mut bid = match self.read_bid(&provided_account_hash)? {
            Some(bid) => bid,
            None => return Err(Error::ValidatorNotFound),
        };

        bid.activate();

        self.write_bid(provided_account_hash, bid)?;

        Ok(())
    }
}
