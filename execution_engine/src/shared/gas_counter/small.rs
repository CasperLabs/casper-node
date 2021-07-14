use super::error::GasLimitError;

/// Gas counter optimized for gas limit smaller or equal to than [`u64::MAX`].
#[derive(Debug, Copy, Clone)]
pub struct SmallGasCounter {
    limit: u64,
    used: u64,
}

impl SmallGasCounter {
    pub(crate) fn new(limit: u64, initial_count: u64) -> Self {
        SmallGasCounter {
            limit,
            used: initial_count,
        }
    }

    /// Add a gas amount.
    pub(crate) fn add(&mut self, additional_gas: u64) -> Result<(), GasLimitError> {
        if self.limit - self.used < additional_gas {
            self.saturate();
            Err(GasLimitError)
        } else {
            // Can't overflow as we already checked this sum is <= `self.limit`
            self.used += additional_gas;
            Ok(())
        }
    }

    /// Saturates gas counter. Any further operation should return [`GasLimitError`].
    pub(crate) fn saturate(&mut self) {
        self.used = self.limit;
    }

    /// Returns currently used gas amount.
    pub(crate) fn used(&self) -> u64 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_small_counter() {
        let mut counter = SmallGasCounter::new(u64::max_value(), 0);
        counter.add(50).unwrap();
        counter.add(100).unwrap();
        counter.add(u64::max_value() - 150).unwrap();
        assert!(counter.add(1).is_err());
        assert!(counter.add(u64::max_value()).is_err());
    }

    #[test]
    fn should_saturate_small_counter() {
        let gas_limit = 1000u64;
        let mut small_counter = SmallGasCounter::new(gas_limit, 0);
        assert!(small_counter.add(gas_limit + 1).is_err());
        assert_eq!(small_counter.used(), gas_limit);
    }

    #[test]
    fn should_saturate_small_counter_2() {
        let gas_limit = u64::MAX;
        let mut small_counter = SmallGasCounter::new(gas_limit, 0);
        small_counter.add(u64::MAX).unwrap();
        assert!(small_counter.add(u64::MAX).is_err());
        assert_eq!(small_counter.used(), gas_limit);
    }
}
