// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use alloc::vec::Vec;
use core::mem::MaybeUninit;

#[cfg(feature = "std")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    bytesrepr::{self, Error, FromBytes, ToBytes},
    U512,
};

const VESTING_SCHEDULE_LENGTH_DAYS: usize = 91;
const DAYS_IN_WEEK: usize = 7;

const WEEK_MILLIS: usize = DAYS_IN_WEEK * 24 * 60 * 60 * 1000;

/// 91 days / 7 days in a week = 13 weeks
const LOCKED_AMOUNTS_LENGTH: usize = (VESTING_SCHEDULE_LENGTH_DAYS / DAYS_IN_WEEK) + 1;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VestingSchedule {
    initial_release_timestamp_millis: u64,
    locked_amounts: Option<[U512; LOCKED_AMOUNTS_LENGTH]>,
}

impl VestingSchedule {
    pub fn new(initial_release_timestamp_millis: u64) -> Self {
        let locked_amounts = None;
        VestingSchedule {
            initial_release_timestamp_millis,
            locked_amounts,
        }
    }

    /// Returns `false` if already initialized.
    pub fn initialize(&mut self, staked_amount: U512) -> bool {
        if self.locked_amounts.is_some() {
            return false;
        }

        let release_period: U512 = U512::from(LOCKED_AMOUNTS_LENGTH);
        let weekly_release = staked_amount / release_period;

        let mut locked_amounts = [U512::zero(); LOCKED_AMOUNTS_LENGTH];
        let mut remaining_locked = staked_amount;

        // Ed and Henry prefer this idiom
        #[allow(clippy::needless_range_loop)]
        for i in 0..LOCKED_AMOUNTS_LENGTH - 1 {
            remaining_locked -= weekly_release;
            locked_amounts[i] = remaining_locked;
        }
        locked_amounts[LOCKED_AMOUNTS_LENGTH - 1] = U512::zero();

        self.locked_amounts = Some(locked_amounts);
        true
    }

    pub fn initial_release_timestamp_millis(&self) -> u64 {
        self.initial_release_timestamp_millis
    }

    pub fn locked_amounts(&self) -> Option<[U512; LOCKED_AMOUNTS_LENGTH]> {
        self.locked_amounts
    }

    pub fn locked_amount(&self, timestamp_millis: u64) -> Option<U512> {
        self.locked_amounts.map(|locked_amounts| {
            assert!(u64::MAX as usize <= usize::MAX);
            let index_timestamp =
                (timestamp_millis - self.initial_release_timestamp_millis) as usize;
            let index = index_timestamp / WEEK_MILLIS;
            if index < LOCKED_AMOUNTS_LENGTH {
                locked_amounts[index]
            } else {
                U512::zero()
            }
        })
    }
}

impl ToBytes for [U512; LOCKED_AMOUNTS_LENGTH] {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        for item in self.iter() {
            result.append(&mut item.to_bytes()?);
        }
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.iter().map(ToBytes::serialized_length).sum::<usize>()
    }
}

impl FromBytes for [U512; LOCKED_AMOUNTS_LENGTH] {
    fn from_bytes(mut bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let mut result: MaybeUninit<[U512; LOCKED_AMOUNTS_LENGTH]> = MaybeUninit::uninit();
        let result_ptr = result.as_mut_ptr() as *mut U512;
        for i in 0..LOCKED_AMOUNTS_LENGTH {
            let (t, remainder) = match FromBytes::from_bytes(bytes) {
                Ok(success) => success,
                Err(error) => {
                    for j in 0..i {
                        unsafe { result_ptr.add(j).drop_in_place() }
                    }
                    return Err(error);
                }
            };
            unsafe { result_ptr.add(i).write(t) };
            bytes = remainder;
        }
        Ok((unsafe { result.assume_init() }, bytes))
    }
}

impl ToBytes for VestingSchedule {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        result.append(&mut self.initial_release_timestamp_millis.to_bytes()?);
        result.append(&mut self.locked_amounts.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.initial_release_timestamp_millis.serialized_length()
            + self.locked_amounts.serialized_length()
    }
}

impl FromBytes for VestingSchedule {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (initial_release_timestamp_millis, bytes) = FromBytes::from_bytes(bytes)?;
        let (locked_amounts, bytes) = FromBytes::from_bytes(bytes)?;
        Ok((
            VestingSchedule {
                initial_release_timestamp_millis,
                locked_amounts,
            },
            bytes,
        ))
    }
}

/// Generators for [`VestingSchedule`]
#[cfg(any(feature = "gens", test))]
pub(crate) mod gens {
    use proptest::{
        array, option,
        prelude::{Arbitrary, Strategy},
    };

    use super::VestingSchedule;
    use crate::gens::u512_arb;

    /// Creates an arbitrary [`VestingSchedule`]
    pub fn vesting_schedule_arb() -> impl Strategy<Value = VestingSchedule> {
        (<u64>::arbitrary(), option::of(array::uniform14(u512_arb()))).prop_map(
            |(initial_release_timestamp_millis, locked_amounts)| VestingSchedule {
                initial_release_timestamp_millis,
                locked_amounts,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prop_assert, proptest};

    use crate::{
        bytesrepr,
        gens::u512_arb,
        system::auction::bid::{
            vesting::{gens::vesting_schedule_arb, LOCKED_AMOUNTS_LENGTH, WEEK_MILLIS},
            VestingSchedule,
        },
        U512,
    };

    /// Default lock-in period of 90 days
    const DEFAULT_LOCKED_FUNDS_PERIOD_MILLIS: u64 = 90 * 24 * 60 * 60 * 1000;

    #[test]
    fn test_locked_amount() {
        const STAKE: u64 = 140;
        const RELEASE_TIMESTAMP: u64 = DEFAULT_LOCKED_FUNDS_PERIOD_MILLIS;
        let mut vesting_schedule = VestingSchedule::new(RELEASE_TIMESTAMP);
        vesting_schedule.initialize(U512::from(STAKE));

        let mut timestamp;

        timestamp = RELEASE_TIMESTAMP;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(130))
        );

        timestamp = RELEASE_TIMESTAMP + WEEK_MILLIS as u64 - 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(130))
        );

        timestamp = RELEASE_TIMESTAMP + WEEK_MILLIS as u64;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(120))
        );

        timestamp = RELEASE_TIMESTAMP + WEEK_MILLIS as u64 + 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(120))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 2) - 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(120))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 2);
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(110))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 2) + 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(110))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 3) - 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(110))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 3);
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(100))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 3) + 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(100))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 12) - 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(20))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 12);
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(10))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 12) + 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(10))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 13) - 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(10))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 13);
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(0))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 13) + 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(0))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 14) - 1;
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(0))
        );

        timestamp = RELEASE_TIMESTAMP + (WEEK_MILLIS as u64 * 14);
        assert_eq!(
            vesting_schedule.locked_amount(timestamp),
            Some(U512::from(0))
        );
    }

    fn vested_amounts_match_initial_stake(initial_stake: U512, release_timestamp: u64) -> bool {
        let mut vesting_schedule = VestingSchedule::new(release_timestamp);
        vesting_schedule.initialize(initial_stake);

        let mut total_vested_amounts = U512::zero();

        for i in 0..LOCKED_AMOUNTS_LENGTH {
            let timestamp = release_timestamp + (WEEK_MILLIS * i) as u64;
            if let Some(locked_amount) = vesting_schedule.locked_amount(timestamp) {
                let current_vested_amount = initial_stake - locked_amount - total_vested_amounts;
                total_vested_amounts += current_vested_amount
            }
        }

        total_vested_amounts == initial_stake
    }

    #[test]
    fn vested_amounts_conserve_stake() {
        let stake = U512::from(1000);
        assert!(vested_amounts_match_initial_stake(
            stake,
            DEFAULT_LOCKED_FUNDS_PERIOD_MILLIS
        ))
    }

    proptest! {
        #[test]
        fn prop_total_vested_amounts_conserve_stake(stake in u512_arb()) {
            prop_assert!(vested_amounts_match_initial_stake(
                stake,
                DEFAULT_LOCKED_FUNDS_PERIOD_MILLIS
            ))
        }

        #[test]
        fn prop_serialization_roundtrip(vesting_schedule in vesting_schedule_arb()) {
            bytesrepr::test_serialization_roundtrip(&vesting_schedule)
        }
    }
}
