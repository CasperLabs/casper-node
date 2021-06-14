#![cfg(test)]

use std::{
    cell::RefCell,
    cmp, env,
    fmt::{self, Debug, Display, Formatter},
    thread,
};

use rand::{self, CryptoRng, Error, Rng, RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;

use casper_types::check_summed_hex;

thread_local! {
    static THIS_THREAD_HAS_RNG: RefCell<bool> = RefCell::new(false);
}

const CL_TEST_SEED: &str = "CL_TEST_SEED";

type Seed = <Pcg64Mcg as SeedableRng>::Seed; // [u8; 16]

/// A fast, seedable pseudorandom number generator for use in tests which prints the seed if the
/// thread in which it is created panics.
///
/// Only one `TestRng` is permitted per thread.
pub struct TestRng {
    seed: Seed,
    rng: Pcg64Mcg,
}

impl TestRng {
    /// Constructs a new `TestRng` using a seed generated from the env var `CL_TEST_SEED` if set or
    /// from cryptographically secure random data if not.
    ///
    /// Note that `new()` or `default()` should only be called once per test.  If a test needs to
    /// spawn multiple threads each with their own `TestRng`, then use `new()` to create a single,
    /// master `TestRng`, then use it to create a seed per child thread.  The child `TestRng`s can
    /// then be constructed in their own threads via `from_seed()`.
    ///
    /// # Panics
    ///
    /// Panics if a `TestRng` has already been created on this thread.
    pub fn new() -> Self {
        Self::set_flag_or_panic();

        let mut seed = Seed::default();
        match env::var(CL_TEST_SEED) {
            Ok(seed_as_hex) => {
                hex::decode_to_slice(&seed_as_hex, &mut seed).unwrap_or_else(|error| {
                    THIS_THREAD_HAS_RNG.with(|flag| {
                        *flag.borrow_mut() = false;
                    });
                    panic!("can't parse '{}' as a TestRng seed: {}", seed_as_hex, error)
                });
            }
            Err(_) => {
                rand::thread_rng().fill(&mut seed);
            }
        };

        let rng = Pcg64Mcg::from_seed(seed);

        TestRng { seed, rng }
    }

    /// Constructs a new `TestRng` using `seed`.  This should be used in cases where a test needs to
    /// spawn multiple threads each with their own `TestRng`.  A single, master `TestRng` should be
    /// constructed before any child threads are spawned, and that one should be used to create
    /// seeds for the child threads' `TestRng`s.
    ///
    /// # Panics
    ///
    /// Panics if a `TestRng` has already been created on this thread.
    pub fn from_seed(seed: Seed) -> Self {
        Self::set_flag_or_panic();
        let rng = Pcg64Mcg::from_seed(seed);
        TestRng { seed, rng }
    }

    fn set_flag_or_panic() {
        THIS_THREAD_HAS_RNG.with(|flag| {
            if *flag.borrow() {
                panic!("cannot create multiple TestRngs on the same thread");
            }
            *flag.borrow_mut() = true;
        });
    }
}

impl Default for TestRng {
    fn default() -> Self {
        TestRng::new()
    }
}

impl Display for TestRng {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(
            formatter,
            "TestRng seed: {}",
            check_summed_hex::encode(&self.seed)
        )
    }
}

impl Debug for TestRng {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        Display::fmt(self, formatter)
    }
}

impl Drop for TestRng {
    fn drop(&mut self) {
        if thread::panicking() {
            let line_1 = format!("Thread: {}", thread::current().name().unwrap_or("unnamed"));
            let line_2 = "To reproduce failure, try running with env var:";
            let line_3 = format!("{}={}", CL_TEST_SEED, check_summed_hex::encode(&self.seed));
            let max_length = cmp::max(line_1.len(), line_2.len());
            let border = "=".repeat(max_length);
            println!(
                "\n{}\n{}\n{}\n{}\n{}\n",
                border, line_1, line_2, line_3, border
            );
        }
    }
}

impl SeedableRng for TestRng {
    type Seed = <Pcg64Mcg as SeedableRng>::Seed;

    fn from_seed(seed: Self::Seed) -> Self {
        Self::from_seed(seed)
    }
}

impl RngCore for TestRng {
    fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.rng.fill_bytes(dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        self.rng.try_fill_bytes(dest)
    }
}

impl CryptoRng for TestRng {}

mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "cannot create multiple TestRngs on the same thread")]
    fn second_test_rng_in_thread_should_panic() {
        let _test_rng1 = TestRng::new();
        let seed = [1; 16];
        let _test_rng2 = TestRng::from_seed(seed);
    }
}
