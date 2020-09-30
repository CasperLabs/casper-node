use std::{
    fmt::{self, Display},
    num::ParseIntError,
    ops::{Add, AddAssign, Div, Mul, Rem, Sub},
    str::FromStr,
    time::{Duration, SystemTime},
};

use datasize::DataSize;
use derive_more::{Add, AddAssign, From, Shl, Shr, Sub, SubAssign};
#[cfg(test)]
use rand::Rng;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::testing::TestRng;

/// A timestamp type, representing a concrete moment in time.
#[derive(
    DataSize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Hash,
    Shr,
    Shl,
)]
pub struct Timestamp(u64);

/// A time difference between two timestamps.
#[derive(
    Debug,
    Clone,
    Copy,
    DataSize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    From,
    Serialize,
    Deserialize,
)]
pub struct TimeDiff(u64);

impl Display for TimeDiff {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TimeDiff {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str(s).map(TimeDiff)
    }
}

impl TimeDiff {
    /// Returns the timestamp as the number of milliseconds since the Unix epoch
    pub fn millis(&self) -> u64 {
        self.0
    }
}

impl Timestamp {
    /// Returns the timestamp of the current moment
    pub fn now() -> Self {
        let millis = SystemTime::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64;
        Timestamp(millis)
    }

    /// Returns a zero timestamp
    pub fn zero() -> Self {
        Timestamp(0)
    }

    /// Returns the timestamp as the number of milliseconds since the Unix epoch
    pub fn millis(&self) -> u64 {
        self.0
    }

    /// Returns the difference between `self` and `other`, or `0` if `self` is earlier than `other`.
    pub fn saturating_sub(self, other: Timestamp) -> TimeDiff {
        TimeDiff(self.0.saturating_sub(other.0))
    }

    /// Returns the number of trailing zeros in the number of milliseconds since the epoch.
    pub fn trailing_zeros(&self) -> u8 {
        self.0.trailing_zeros() as u8
    }

    /// Generates a random instance using a `TestRng`.
    #[cfg(test)]
    pub fn random(rng: &mut TestRng) -> Self {
        Timestamp(1_596_763_000_000 + rng.gen_range(200_000, 1_000_000))
    }
}

impl Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Timestamp {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str(s).map(Timestamp)
    }
}

impl Sub<Timestamp> for Timestamp {
    type Output = TimeDiff;

    fn sub(self, other: Timestamp) -> TimeDiff {
        TimeDiff(self.0 - other.0)
    }
}

impl Add<TimeDiff> for Timestamp {
    type Output = Timestamp;

    fn add(self, diff: TimeDiff) -> Timestamp {
        Timestamp(self.0 + diff.0)
    }
}

impl AddAssign<TimeDiff> for Timestamp {
    fn add_assign(&mut self, rhs: TimeDiff) {
        self.0 += rhs.0;
    }
}

impl Sub<TimeDiff> for Timestamp {
    type Output = Timestamp;

    fn sub(self, diff: TimeDiff) -> Timestamp {
        Timestamp(self.0 - diff.0)
    }
}

impl Div<TimeDiff> for Timestamp {
    type Output = u64;

    fn div(self, rhs: TimeDiff) -> u64 {
        self.0 / rhs.0
    }
}

impl Rem<TimeDiff> for Timestamp {
    type Output = TimeDiff;

    fn rem(self, diff: TimeDiff) -> TimeDiff {
        TimeDiff(self.0 % diff.0)
    }
}

impl Mul<u64> for TimeDiff {
    type Output = TimeDiff;

    fn mul(self, rhs: u64) -> TimeDiff {
        TimeDiff(self.0 * rhs)
    }
}

impl Div<u64> for TimeDiff {
    type Output = TimeDiff;

    fn div(self, rhs: u64) -> TimeDiff {
        TimeDiff(self.0 / rhs)
    }
}

impl From<TimeDiff> for Duration {
    fn from(diff: TimeDiff) -> Duration {
        Duration::from_millis(diff.0)
    }
}

#[cfg(test)]
impl From<Duration> for TimeDiff {
    fn from(duration: Duration) -> TimeDiff {
        TimeDiff(duration.as_millis() as u64)
    }
}

#[cfg(test)]
impl From<u64> for Timestamp {
    fn from(arg: u64) -> Timestamp {
        Timestamp(arg)
    }
}
