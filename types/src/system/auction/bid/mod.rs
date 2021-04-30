// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

mod vesting;

use alloc::{collections::BTreeMap, vec::Vec};

#[cfg(feature = "std")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    bytesrepr::{self, FromBytes, ToBytes},
    system::auction::{DelegationRate, Delegator, Error},
    CLType, CLTyped, PublicKey, URef, U512,
};

pub use vesting::VestingSchedule;

/// An entry in the validator map.
#[derive(PartialEq, Eq, Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Bid {
    /// Validator public key
    validator_public_key: PublicKey,
    /// The purse that was used for bonding.
    bonding_purse: URef,
    /// The amount of tokens staked by a validator (not including delegators).
    staked_amount: U512,
    /// Delegation rate
    delegation_rate: DelegationRate,
    /// Vesting schedule for a genesis validator. `None` if non-genesis validator.
    vesting_schedule: Option<VestingSchedule>,
    /// This validator's delegators, indexed by their public keys
    delegators: BTreeMap<PublicKey, Delegator>,
    /// `true` if validator has been "evicted"
    inactive: bool,
}

impl Bid {
    /// Creates new instance of a bid with locked funds.
    pub fn locked(
        validator_public_key: PublicKey,
        bonding_purse: URef,
        staked_amount: U512,
        delegation_rate: DelegationRate,
        release_timestamp_millis: u64,
    ) -> Self {
        let vesting_schedule = Some(VestingSchedule::new(release_timestamp_millis));
        let delegators = BTreeMap::new();
        let inactive = false;
        Self {
            validator_public_key,
            bonding_purse,
            staked_amount,
            delegation_rate,
            vesting_schedule,
            delegators,
            inactive,
        }
    }

    /// Creates new instance of a bid with unlocked funds.
    pub fn unlocked(
        validator_public_key: PublicKey,
        bonding_purse: URef,
        staked_amount: U512,
        delegation_rate: DelegationRate,
    ) -> Self {
        let vesting_schedule = None;
        let delegators = BTreeMap::new();
        let inactive = false;
        Self {
            validator_public_key,
            bonding_purse,
            staked_amount,
            delegation_rate,
            vesting_schedule,
            delegators,
            inactive,
        }
    }

    /// Creates a new inactive instance of a bid with 0 staked amount.
    pub fn empty(validator_public_key: PublicKey, bonding_purse: URef) -> Self {
        let vesting_schedule = None;
        let delegators = BTreeMap::new();
        let inactive = true;
        let staked_amount = 0.into();
        let delegation_rate = Default::default();
        Self {
            validator_public_key,
            bonding_purse,
            staked_amount,
            delegation_rate,
            vesting_schedule,
            delegators,
            inactive,
        }
    }

    /// Gets the validator public key of the provided bid
    pub fn validator_public_key(&self) -> &PublicKey {
        &self.validator_public_key
    }

    /// Gets the bonding purse of the provided bid
    pub fn bonding_purse(&self) -> &URef {
        &self.bonding_purse
    }

    /// Gets the staked amount of the provided bid
    pub fn staked_amount(&self) -> &U512 {
        &self.staked_amount
    }

    /// Gets the staked amount of the provided bid
    pub fn staked_amount_mut(&mut self) -> &mut U512 {
        &mut self.staked_amount
    }

    /// Gets the delegation rate of the provided bid
    pub fn delegation_rate(&self) -> &DelegationRate {
        &self.delegation_rate
    }

    /// Returns a reference to the vesting schedule of the provided bid.  `None` if a non-genesis
    /// validator.
    pub fn vesting_schedule(&self) -> Option<&VestingSchedule> {
        self.vesting_schedule.as_ref()
    }

    /// Returns a mutable reference to the vesting schedule of the provided bid.  `None` if a
    /// non-genesis validator.
    pub fn vesting_schedule_mut(&mut self) -> Option<&mut VestingSchedule> {
        self.vesting_schedule.as_mut()
    }

    /// Returns a reference to the delegators of the provided bid
    pub fn delegators(&self) -> &BTreeMap<PublicKey, Delegator> {
        &self.delegators
    }

    /// Returns a mutable reference to the delegators of the provided bid
    pub fn delegators_mut(&mut self) -> &mut BTreeMap<PublicKey, Delegator> {
        &mut self.delegators
    }

    /// Returns `true` if validator is inactive
    pub fn inactive(&self) -> bool {
        self.inactive
    }

    /// Decreases the stake of the provided bid
    pub fn decrease_stake(
        &mut self,
        amount: U512,
        era_end_timestamp_millis: u64,
    ) -> Result<U512, Error> {
        let updated_staked_amount = self
            .staked_amount
            .checked_sub(amount)
            .ok_or(Error::UnbondTooLarge)?;

        let vesting_schedule = match self.vesting_schedule.as_ref() {
            Some(vesting_schedule) => vesting_schedule,
            None => {
                self.staked_amount = updated_staked_amount;
                return Ok(updated_staked_amount);
            }
        };

        match vesting_schedule.locked_amount(era_end_timestamp_millis) {
            Some(locked_amount) if updated_staked_amount < locked_amount => {
                Err(Error::ValidatorFundsLocked)
            }
            None => {
                // If `None`, then the locked amounts table has yet to be initialized (likely
                // pre-90 day mark)
                Err(Error::ValidatorFundsLocked)
            }
            Some(_) => {
                self.staked_amount = updated_staked_amount;
                Ok(updated_staked_amount)
            }
        }
    }

    /// Increases the stake of the provided bid
    pub fn increase_stake(&mut self, amount: U512) -> Result<U512, Error> {
        let updated_staked_amount = self
            .staked_amount
            .checked_add(amount)
            .ok_or(Error::InvalidAmount)?;

        self.staked_amount = updated_staked_amount;

        Ok(updated_staked_amount)
    }

    /// Updates the delegation rate of the provided bid
    pub fn with_delegation_rate(&mut self, delegation_rate: DelegationRate) -> &mut Self {
        self.delegation_rate = delegation_rate;
        self
    }

    /// Initializes the vesting schedule of provided bid if the provided timestamp is greater than
    /// or equal to the bid's initial release timestamp and the bid is owned by a genesis
    /// validator.
    ///
    /// Returns `true` if the provided bid's vesting schedule was initialized.
    pub fn process(&mut self, timestamp_millis: u64) -> bool {
        // Put timestamp-sensitive processing logic in here
        let staked_amount = self.staked_amount;
        let vesting_schedule = match self.vesting_schedule_mut() {
            Some(vesting_schedule) => vesting_schedule,
            None => return false,
        };
        if timestamp_millis < vesting_schedule.initial_release_timestamp_millis() {
            return false;
        }

        let mut initialized = false;

        if vesting_schedule.initialize(staked_amount) {
            initialized = true;
        }

        for delegator in self.delegators_mut().values_mut() {
            let staked_amount = *delegator.staked_amount();
            if let Some(vesting_schedule) = delegator.vesting_schedule_mut() {
                if timestamp_millis >= vesting_schedule.initial_release_timestamp_millis()
                    && vesting_schedule.initialize(staked_amount)
                {
                    initialized = true;
                }
            }
        }

        initialized
    }

    /// Sets given bid's `inactive` field to `false`
    pub fn activate(&mut self) -> bool {
        self.inactive = false;
        false
    }

    /// Sets given bid's `inactive` field to `true`
    pub fn deactivate(&mut self) -> bool {
        self.inactive = true;
        true
    }

    /// Returns the total staked amount of validator + all delegators
    pub fn total_staked_amount(&self) -> Result<U512, Error> {
        self.delegators
            .iter()
            .fold(Some(U512::zero()), |maybe_a, (_, b)| {
                maybe_a.and_then(|a| a.checked_add(*b.staked_amount()))
            })
            .and_then(|delegators_sum| delegators_sum.checked_add(*self.staked_amount()))
            .ok_or(Error::InvalidAmount)
    }
}

impl CLTyped for Bid {
    fn cl_type() -> CLType {
        CLType::Any
    }
}

impl ToBytes for Bid {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        result.extend(self.validator_public_key.to_bytes()?);
        result.extend(self.bonding_purse.to_bytes()?);
        result.extend(self.staked_amount.to_bytes()?);
        result.extend(self.delegation_rate.to_bytes()?);
        result.extend(self.vesting_schedule.to_bytes()?);
        result.extend(self.delegators.to_bytes()?);
        result.extend(self.inactive.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.validator_public_key.serialized_length()
            + self.bonding_purse.serialized_length()
            + self.staked_amount.serialized_length()
            + self.delegation_rate.serialized_length()
            + self.vesting_schedule.serialized_length()
            + self.delegators.serialized_length()
            + self.inactive.serialized_length()
    }
}

impl FromBytes for Bid {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (validator_public_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (bonding_purse, bytes) = FromBytes::from_bytes(bytes)?;
        let (staked_amount, bytes) = FromBytes::from_bytes(bytes)?;
        let (delegation_rate, bytes) = FromBytes::from_bytes(bytes)?;
        let (vesting_schedule, bytes) = FromBytes::from_bytes(bytes)?;
        let (delegators, bytes) = FromBytes::from_bytes(bytes)?;
        let (inactive, bytes) = FromBytes::from_bytes(bytes)?;
        Ok((
            Bid {
                validator_public_key,
                bonding_purse,
                staked_amount,
                delegation_rate,
                vesting_schedule,
                delegators,
                inactive,
            },
            bytes,
        ))
    }
}

#[cfg(any(feature = "gens", test))]
pub(crate) mod gens {
    use super::Bid;
    pub use crate::system::auction::bid::vesting::gens::*;
    use crate::{
        crypto::gens::public_key_arb,
        gens::{u512_arb, uref_arb},
        system::auction::{gens::delegators_arb, DelegationRate},
    };
    use proptest::{option, prelude::*};

    prop_compose! {
        /// Creates an arbitrary [`Bid`]
        pub fn bid_arb()(validator_public_key in public_key_arb(),
                         bonding_purse in uref_arb(),
                         staked_amount in u512_arb(),
                         delegation_rate: DelegationRate,
                         vesting_schedule in option::of(vesting_schedule_arb()),
                         delegators in delegators_arb(),
                         inactive: bool) -> Bid {
            Bid {
                validator_public_key,
                bonding_purse,
                staked_amount,
                delegation_rate,
                vesting_schedule,
                delegators,
                inactive,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use crate::{
        bytesrepr,
        system::auction::{bid::VestingSchedule, Bid, DelegationRate, Delegator},
        AccessRights, PublicKey, SecretKey, URef, U512,
    };

    #[test]
    fn serialization_roundtrip() {
        let founding_validator = Bid {
            validator_public_key: PublicKey::from(
                &SecretKey::ed25519_from_bytes([0u8; SecretKey::ED25519_LENGTH]).unwrap(),
            ),
            bonding_purse: URef::new([42; 32], AccessRights::READ_ADD_WRITE),
            staked_amount: U512::one(),
            delegation_rate: DelegationRate::max_value(),
            vesting_schedule: Some(VestingSchedule::default()),
            delegators: BTreeMap::default(),
            inactive: true,
        };
        bytesrepr::test_serialization_roundtrip(&founding_validator);
    }

    #[test]
    fn should_initialize_delegators_different_timestamps() {
        const WEEK_MILLIS: u64 = 7 * 24 * 60 * 60 * 1000;

        const TIMESTAMP_MILLIS: u64 = WEEK_MILLIS as u64;

        let validator_pk: PublicKey = (&SecretKey::ed25519_from_bytes([42; 32]).unwrap()).into();

        let delegator_1_pk: PublicKey = (&SecretKey::ed25519_from_bytes([43; 32]).unwrap()).into();
        let delegator_2_pk: PublicKey = (&SecretKey::ed25519_from_bytes([44; 32]).unwrap()).into();

        let validator_release_timestamp = TIMESTAMP_MILLIS;
        let validator_bonding_purse = URef::new([42; 32], AccessRights::ADD);
        let validator_staked_amount = U512::from(1000);
        let validator_delegation_rate = 0;

        let delegator_1_release_timestamp = TIMESTAMP_MILLIS + 1;
        let delegator_1_bonding_purse = URef::new([52; 32], AccessRights::ADD);
        let delegator_1_staked_amount = U512::from(2000);

        let delegator_2_release_timestamp = TIMESTAMP_MILLIS + 2;
        let delegator_2_bonding_purse = URef::new([62; 32], AccessRights::ADD);
        let delegator_2_staked_amount = U512::from(3000);

        let delegator_1 = Delegator::locked(
            delegator_1_pk.clone(),
            delegator_1_staked_amount,
            delegator_1_bonding_purse,
            validator_pk.clone(),
            delegator_1_release_timestamp,
        );

        let delegator_2 = Delegator::locked(
            delegator_2_pk.clone(),
            delegator_2_staked_amount,
            delegator_2_bonding_purse,
            validator_pk.clone(),
            delegator_2_release_timestamp,
        );

        let mut bid = Bid::locked(
            validator_pk,
            validator_bonding_purse,
            validator_staked_amount,
            validator_delegation_rate,
            validator_release_timestamp,
        );

        assert!(!bid.process(validator_release_timestamp - 1));

        {
            let delegators = bid.delegators_mut();

            delegators.insert(delegator_1_pk.clone(), delegator_1);
            delegators.insert(delegator_2_pk.clone(), delegator_2);
        }

        assert!(bid.process(delegator_1_release_timestamp));

        let delegator_1_updated_1 = bid.delegators().get(&delegator_1_pk).cloned().unwrap();
        assert!(delegator_1_updated_1
            .vesting_schedule()
            .unwrap()
            .locked_amounts()
            .is_some());

        let delegator_2_updated_1 = bid.delegators().get(&delegator_2_pk).cloned().unwrap();
        assert!(delegator_2_updated_1
            .vesting_schedule()
            .unwrap()
            .locked_amounts()
            .is_none());

        assert!(bid.process(delegator_2_release_timestamp));

        let delegator_1_updated_2 = bid.delegators().get(&delegator_1_pk).cloned().unwrap();
        assert!(delegator_1_updated_2
            .vesting_schedule()
            .unwrap()
            .locked_amounts()
            .is_some());
        // Delegator 1 is already initialized and did not change after 2nd Bid::process
        assert_eq!(delegator_1_updated_1, delegator_1_updated_2);

        let delegator_2_updated_2 = bid.delegators().get(&delegator_2_pk).cloned().unwrap();
        assert!(delegator_2_updated_2
            .vesting_schedule()
            .unwrap()
            .locked_amounts()
            .is_some());

        // Delegator 2 is different compared to first Bid::process
        assert_ne!(delegator_2_updated_1, delegator_2_updated_2);

        // Validator initialized, and all delegators initialized
        assert!(!bid.process(delegator_2_release_timestamp + 1));
    }
}
