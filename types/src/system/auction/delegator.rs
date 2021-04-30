// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use alloc::vec::Vec;

#[cfg(feature = "std")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    bytesrepr::{self, FromBytes, ToBytes},
    system::auction::{bid::VestingSchedule, Error},
    CLType, CLTyped, PublicKey, URef, U512,
};

/// Represents a party delegating their stake to a validator (or "delegatee")
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Delegator {
    delegator_public_key: PublicKey,
    staked_amount: U512,
    bonding_purse: URef,
    validator_public_key: PublicKey,
    vesting_schedule: Option<VestingSchedule>,
}

impl Delegator {
    /// Creates a new [`Delegator`]
    pub fn unlocked(
        delegator_public_key: PublicKey,
        staked_amount: U512,
        bonding_purse: URef,
        validator_public_key: PublicKey,
    ) -> Self {
        let vesting_schedule = None;
        Delegator {
            delegator_public_key,
            staked_amount,
            bonding_purse,
            validator_public_key,
            vesting_schedule,
        }
    }

    /// Creates new instance of a [`Delegator`] with locked funds.
    pub fn locked(
        delegator_public_key: PublicKey,
        staked_amount: U512,
        bonding_purse: URef,
        validator_public_key: PublicKey,
        release_timestamp_millis: u64,
    ) -> Self {
        let vesting_schedule = Some(VestingSchedule::new(release_timestamp_millis));
        Delegator {
            delegator_public_key,
            staked_amount,
            bonding_purse,
            validator_public_key,
            vesting_schedule,
        }
    }

    /// Returns the staked amount
    pub fn staked_amount(&self) -> &U512 {
        &self.staked_amount
    }

    /// Returns the mutable staked amount
    pub fn staked_amount_mut(&mut self) -> &mut U512 {
        &mut self.staked_amount
    }

    /// Returns the bonding purse
    pub fn bonding_purse(&self) -> &URef {
        &self.bonding_purse
    }

    /// Returns delegatee
    pub fn validator_public_key(&self) -> &PublicKey {
        &self.validator_public_key
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
            .ok_or(Error::InvalidAmount)?;

        let vesting_schedule = match self.vesting_schedule.as_ref() {
            Some(vesting_schedule) => vesting_schedule,
            None => {
                self.staked_amount = updated_staked_amount;
                return Ok(updated_staked_amount);
            }
        };

        match vesting_schedule.locked_amount(era_end_timestamp_millis) {
            Some(locked_amount) if updated_staked_amount < locked_amount => {
                Err(Error::DelegatorFundsLocked)
            }
            None => {
                // If `None`, then the locked amounts table has yet to be initialized (likely
                // pre-90 day mark)
                Err(Error::DelegatorFundsLocked)
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

    /// Returns a reference to the vesting schedule of the provided
    /// delegator bid.  `None` if a non-genesis validator.
    pub fn vesting_schedule(&self) -> Option<&VestingSchedule> {
        self.vesting_schedule.as_ref()
    }

    /// Returns a mutable reference to the vesting schedule of the provided
    /// delegator bid.  `None` if a non-genesis validator.
    pub fn vesting_schedule_mut(&mut self) -> Option<&mut VestingSchedule> {
        self.vesting_schedule.as_mut()
    }
}

impl CLTyped for Delegator {
    fn cl_type() -> CLType {
        CLType::Any
    }
}

impl ToBytes for Delegator {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut buffer = bytesrepr::allocate_buffer(self)?;
        buffer.extend(self.delegator_public_key.to_bytes()?);
        buffer.extend(self.staked_amount.to_bytes()?);
        buffer.extend(self.bonding_purse.to_bytes()?);
        buffer.extend(self.validator_public_key.to_bytes()?);
        buffer.extend(self.vesting_schedule.to_bytes()?);
        Ok(buffer)
    }

    fn serialized_length(&self) -> usize {
        self.delegator_public_key.serialized_length()
            + self.staked_amount.serialized_length()
            + self.bonding_purse.serialized_length()
            + self.validator_public_key.serialized_length()
            + self.vesting_schedule.serialized_length()
    }
}

impl FromBytes for Delegator {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (delegator_public_key, bytes) = PublicKey::from_bytes(bytes)?;
        let (staked_amount, bytes) = U512::from_bytes(bytes)?;
        let (bonding_purse, bytes) = URef::from_bytes(bytes)?;
        let (validator_public_key, bytes) = PublicKey::from_bytes(bytes)?;
        let (vesting_schedule, bytes) = FromBytes::from_bytes(bytes)?;
        Ok((
            Delegator {
                delegator_public_key,
                staked_amount,
                bonding_purse,
                validator_public_key,
                vesting_schedule,
            },
            bytes,
        ))
    }
}

#[cfg(any(feature = "gens", test))]
pub(crate) mod gens {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        crypto::gens::public_key_arb,
        gens::{u512_arb, uref_arb},
        system::auction::bid::gens::vesting_schedule_arb,
    };
    use proptest::{collection::btree_map, option, prelude::*};

    prop_compose! {
        /// Creates ca
        pub fn delegator_arb()(delegator_public_key in public_key_arb(),
                               staked_amount in u512_arb(),
                               bonding_purse in uref_arb(),
                               validator_public_key in public_key_arb(),
                               vesting_schedule in option::of(vesting_schedule_arb())) -> Delegator {
            Delegator {
                delegator_public_key,
                staked_amount,
                bonding_purse,
                validator_public_key,
                vesting_schedule,
            }
        }
    }

    /// Creates an arbitrary delegators mapping.
    pub fn delegators_arb() -> impl Strategy<Value = BTreeMap<PublicKey, Delegator>> {
        btree_map(public_key_arb(), delegator_arb(), 0..10)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        bytesrepr, system::auction::Delegator, AccessRights, PublicKey, SecretKey, URef, U512,
    };

    #[test]
    fn serialization_roundtrip() {
        let staked_amount = U512::one();
        let bonding_purse = URef::new([42; 32], AccessRights::READ_ADD_WRITE);
        let delegator_public_key: PublicKey = PublicKey::from(
            &SecretKey::ed25519_from_bytes([42; SecretKey::ED25519_LENGTH]).unwrap(),
        );

        let validator_public_key: PublicKey = PublicKey::from(
            &SecretKey::ed25519_from_bytes([43; SecretKey::ED25519_LENGTH]).unwrap(),
        );
        let unlocked_delegator = Delegator::unlocked(
            delegator_public_key.clone(),
            staked_amount,
            bonding_purse,
            validator_public_key.clone(),
        );
        bytesrepr::test_serialization_roundtrip(&unlocked_delegator);

        let release_timestamp_millis = 42;
        let locked_delegator = Delegator::locked(
            delegator_public_key,
            staked_amount,
            bonding_purse,
            validator_public_key,
            release_timestamp_millis,
        );
        bytesrepr::test_serialization_roundtrip(&locked_delegator);
    }
}
