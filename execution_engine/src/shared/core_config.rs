use datasize::DataSize;
use rand::{distributions::Standard, prelude::*, Rng};
use serde::{Deserialize, Serialize};

use casper_types::bytesrepr::{self, FromBytes, ToBytes};

use crate::storage::protocol_data::DEFAULT_MAX_ASSOCIATED_KEYS;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, DataSize)]
pub struct CoreConfig {
    /// Maximum number of associated keys (i.e. map of
    /// [`AccountHash`](casper_types::account::AccountHash)s to
    /// [`Weight`](casper_types::account::Weight)s) for a single account.
    max_associated_keys: u32,
}

impl CoreConfig {
    pub fn new(max_associated_keys: u32) -> Self {
        Self {
            max_associated_keys,
        }
    }

    pub fn max_associated_keys(&self) -> u32 {
        self.max_associated_keys
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        CoreConfig {
            max_associated_keys: DEFAULT_MAX_ASSOCIATED_KEYS,
        }
    }
}

impl Distribution<CoreConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> CoreConfig {
        CoreConfig {
            max_associated_keys: rng.gen(),
        }
    }
}

impl ToBytes for CoreConfig {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut ret = bytesrepr::unchecked_allocate_buffer(self);

        ret.append(&mut self.max_associated_keys.to_bytes()?);

        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.max_associated_keys.serialized_length()
    }
}

impl FromBytes for CoreConfig {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (max_associated_keys, rem) = FromBytes::from_bytes(bytes)?;

        Ok((
            CoreConfig {
                max_associated_keys,
            },
            rem,
        ))
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::{num, prop_compose};

    use super::CoreConfig;

    prop_compose! {
        pub fn core_config_arb() (
            max_associated_keys in num::u32::ANY,
        ) -> CoreConfig {
            CoreConfig {
                max_associated_keys,
            }
        }
    }
}
