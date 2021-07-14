use std::{fmt, iter::Sum};

use num::Zero;

use casper_types::U512;

use crate::shared::motes::Motes;

#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Gas(U512);

impl Gas {
    pub const fn new(value: U512) -> Self {
        Gas(value)
    }

    pub fn value(&self) -> U512 {
        self.0
    }

    pub fn from_motes(motes: Motes, conv_rate: u64) -> Option<Self> {
        motes
            .value()
            .checked_div(U512::from(conv_rate))
            .map(Self::new)
    }

    pub fn checked_add(&self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.value()).map(Self::new)
    }

    pub fn checked_sub(&self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.value()).map(Self::new)
    }
}

impl fmt::Display for Gas {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl std::ops::Add for Gas {
    type Output = Gas;

    fn add(self, rhs: Self) -> Self::Output {
        let val = self.value() + rhs.value();
        Gas::new(val)
    }
}

impl std::ops::Sub for Gas {
    type Output = Gas;

    fn sub(self, rhs: Self) -> Self::Output {
        let val = self.value() - rhs.value();
        Gas::new(val)
    }
}

impl std::ops::Div for Gas {
    type Output = Gas;

    fn div(self, rhs: Self) -> Self::Output {
        let val = self.value() / rhs.value();
        Gas::new(val)
    }
}

impl std::ops::Mul for Gas {
    type Output = Gas;

    fn mul(self, rhs: Self) -> Self::Output {
        let val = self.value() * rhs.value();
        Gas::new(val)
    }
}

impl std::ops::AddAssign for Gas {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0
    }
}

impl Zero for Gas {
    fn zero() -> Self {
        Gas::new(U512::zero())
    }

    fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
}

impl Sum for Gas {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Gas::zero(), std::ops::Add::add)
    }
}

impl From<u32> for Gas {
    fn from(gas: u32) -> Self {
        let gas_u512: U512 = gas.into();
        Gas::new(gas_u512)
    }
}

impl From<u64> for Gas {
    fn from(gas: u64) -> Self {
        let gas_u512: U512 = gas.into();
        Gas::new(gas_u512)
    }
}

#[cfg(test)]
mod tests {
    use casper_types::U512;

    use crate::shared::{gas::Gas, motes::Motes};

    #[test]
    fn should_be_able_to_get_instance_of_gas() {
        let initial_value = 1;
        let gas = Gas::new(U512::from(initial_value));
        assert_eq!(
            initial_value,
            gas.value().as_u64(),
            "should have equal value"
        )
    }

    #[test]
    fn should_be_able_to_compare_two_instances_of_gas() {
        let left_gas = Gas::new(U512::from(1));
        let right_gas = Gas::new(U512::from(1));
        assert_eq!(left_gas, right_gas, "should be equal");
        let right_gas = Gas::new(U512::from(2));
        assert_ne!(left_gas, right_gas, "should not be equal")
    }

    #[test]
    fn should_be_able_to_add_two_instances_of_gas() {
        let left_gas = Gas::new(U512::from(1));
        let right_gas = Gas::new(U512::from(1));
        let expected_gas = Gas::new(U512::from(2));
        assert_eq!((left_gas + right_gas), expected_gas, "should be equal")
    }

    #[test]
    fn should_be_able_to_subtract_two_instances_of_gas() {
        let left_gas = Gas::new(U512::from(1));
        let right_gas = Gas::new(U512::from(1));
        let expected_gas = Gas::new(U512::from(0));
        assert_eq!((left_gas - right_gas), expected_gas, "should be equal")
    }

    #[test]
    fn should_be_able_to_multiply_two_instances_of_gas() {
        let left_gas = Gas::new(U512::from(100));
        let right_gas = Gas::new(U512::from(10));
        let expected_gas = Gas::new(U512::from(1000));
        assert_eq!((left_gas * right_gas), expected_gas, "should be equal")
    }

    #[test]
    fn should_be_able_to_divide_two_instances_of_gas() {
        let left_gas = Gas::new(U512::from(1000));
        let right_gas = Gas::new(U512::from(100));
        let expected_gas = Gas::new(U512::from(10));
        assert_eq!((left_gas / right_gas), expected_gas, "should be equal")
    }

    #[test]
    fn should_be_able_to_convert_from_mote() {
        let mote = Motes::new(U512::from(100));
        let gas = Gas::from_motes(mote, 10).expect("should have gas");
        let expected_gas = Gas::new(U512::from(10));
        assert_eq!(gas, expected_gas, "should be equal")
    }

    #[test]
    fn should_be_able_to_default() {
        let gas = Gas::default();
        let expected_gas = Gas::new(U512::from(0));
        assert_eq!(gas, expected_gas, "should be equal")
    }

    #[test]
    fn should_be_able_to_compare_relative_value() {
        let left_gas = Gas::new(U512::from(100));
        let right_gas = Gas::new(U512::from(10));
        assert!(left_gas > right_gas, "should be gt");
        let right_gas = Gas::new(U512::from(100));
        assert!(left_gas >= right_gas, "should be gte");
        assert!(left_gas <= right_gas, "should be lte");
        let left_gas = Gas::new(U512::from(10));
        assert!(left_gas < right_gas, "should be lt");
    }

    #[test]
    fn should_default() {
        let left_gas = Gas::new(U512::from(0));
        let right_gas = Gas::default();
        assert_eq!(left_gas, right_gas, "should be equal");
        let u512 = U512::zero();
        assert_eq!(left_gas.value(), u512, "should be equal");
    }

    #[test]
    fn should_support_checked_div_from_motes() {
        let motes = Motes::new(U512::zero());
        let conv_rate = 0;
        let maybe = Gas::from_motes(motes, conv_rate);
        assert!(maybe.is_none(), "should be none due to divide by zero");
    }
}
