pub mod error;
mod large;
mod small;

use large::{LargeGasCounter, U64_MAX_AS_U512};
use small::SmallGasCounter;

use self::error::GasLimitError;

use super::gas::Gas;

/// Implements a gas counter functionality which uses specific implementation dependent on the size
/// of gas limit to speed things up.
#[derive(Debug, Copy, Clone)]
pub enum GasCounter {
    Small(SmallGasCounter),
    Large(LargeGasCounter),
}

impl GasCounter {
    /// Create a new gas counter with provided initial values.
    pub fn new(limit: Gas, initial_count: Gas) -> GasCounter {
        if limit.value() <= U64_MAX_AS_U512 && initial_count.value() <= U64_MAX_AS_U512 {
            // Small counter if values fit in `0..u64::MAX`
            GasCounter::Small(SmallGasCounter::new(
                limit.value().as_u64(),
                initial_count.value().as_u64(),
            ))
        } else {
            GasCounter::Large(LargeGasCounter::new(limit, initial_count))
        }
    }

    /// Adds a gas charge.
    pub fn add(&mut self, additional_gas: u64) -> Result<(), GasLimitError> {
        match self {
            GasCounter::Small(counter) => counter.add(additional_gas),
            GasCounter::Large(counter) => counter.add(additional_gas),
        }
    }

    /// Adds a large gas amount (possibly larger than `u64::MAX`). This comes from sources where gas
    /// cost is multiplied from multiple ingredients which possibly could extend the supported range
    /// of [`GasCounter::add`].
    pub fn add_large(&mut self, additional_gas: Gas) -> Result<(), GasLimitError> {
        let gas_value = additional_gas.value();
        if gas_value <= U64_MAX_AS_U512 {
            self.add(gas_value.as_u64())
        } else {
            match self {
                GasCounter::Small(small) => {
                    // Adding anything larger than `u64::MAX` into small counter should instantly
                    // saturate the counter.
                    small.saturate();
                    Err(GasLimitError)
                }
                GasCounter::Large(large) => large.add_large(gas_value),
            }
        }
    }

    /// Returns how much has is currently used.
    pub fn used(&self) -> Gas {
        match self {
            GasCounter::Small(small) => small.used().into(),
            GasCounter::Large(large) => Gas::new(large.used()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::large::U64_MAX_AS_U512;

    use casper_types::U512;

    #[test]
    fn should_create_valid_counters() {
        let small_counter = GasCounter::new(Gas::new(U64_MAX_AS_U512), Gas::new(U512::zero()));
        assert!(matches!(small_counter, GasCounter::Small(_)));

        let big_counter = GasCounter::new(
            Gas::new(U64_MAX_AS_U512 + U512::one()),
            Gas::new(U512::zero()),
        );
        assert!(matches!(big_counter, GasCounter::Large(_)));
    }

    #[test]
    fn should_not_add_large_cost() {
        let gas_limit = Gas::new(U512::from(1000));
        let mut small_counter = GasCounter::new(gas_limit, Gas::new(U512::zero()));
        assert!(matches!(small_counter, GasCounter::Small(_)));
        assert!(small_counter
            .add_large(Gas::new(U64_MAX_AS_U512 + U512::one()))
            .is_err());
        assert_eq!(small_counter.used(), gas_limit);
    }

    #[test]
    fn should_saturate_small_counter() {
        let gas_limit = 1000u64;
        let mut small_counter = GasCounter::new(gas_limit.into(), Gas::new(U512::zero()));
        assert!(small_counter.add(gas_limit + 1).is_err());
        assert_eq!(small_counter.used(), gas_limit.into());
    }

    #[test]
    fn should_saturate_large_counter() {
        let gas_limit = U64_MAX_AS_U512;
        let mut large_counter = GasCounter::new(Gas::new(gas_limit), Gas::new(U512::zero()));
        assert!(large_counter
            .add_large(Gas::new(gas_limit + U512::one()))
            .is_err());
        assert_eq!(large_counter.used(), Gas::new(gas_limit));
    }
}
