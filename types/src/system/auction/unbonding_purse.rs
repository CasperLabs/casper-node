// TODO - remove once schemars stops causing warning.
#![allow(clippy::field_reassign_with_default)]

use alloc::vec::Vec;

#[cfg(feature = "std")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    bytesrepr::{self, FromBytes, ToBytes},
    CLType, CLTyped, EraId, PublicKey, URef, U512,
};

/// Unbonding purse.
#[derive(PartialEq, Eq, Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "std", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct UnbondingPurse {
    /// Bonding Purse
    bonding_purse: URef,
    /// Validators public key.
    validator_public_key: PublicKey,
    /// Unbonders public key.
    unbonder_public_key: PublicKey,
    /// Era in which this unbonding request was created.
    era_of_creation: EraId,
    /// Unbonding Amount.
    amount: U512,
}

impl UnbondingPurse {
    /// Creates [`UnbondingPurse`] instance for an unbonding request.
    pub const fn new(
        bonding_purse: URef,
        validator_public_key: PublicKey,
        unbonder_public_key: PublicKey,
        era_of_creation: EraId,
        amount: U512,
    ) -> Self {
        Self {
            bonding_purse,
            validator_public_key,
            unbonder_public_key,
            era_of_creation,
            amount,
        }
    }

    /// Checks if given request is made by a validator by checking if public key of unbonder is same
    /// as a key owned by validator.
    pub fn is_validator(&self) -> bool {
        self.validator_public_key == self.unbonder_public_key
    }

    /// Returns bonding purse used to make this unbonding request.
    pub fn bonding_purse(&self) -> &URef {
        &self.bonding_purse
    }

    /// Returns public key of validator.
    pub fn validator_public_key(&self) -> &PublicKey {
        &self.validator_public_key
    }

    /// Returns public key of unbonder.
    ///
    /// For withdrawal requests that originated from validator's public key through
    /// [`crate::system::auction::Auction::withdraw_bid`] entrypoint this is equal to
    /// [`UnbondingPurse::validator_public_key`] and [`UnbondingPurse::is_validator`] is `true`.
    pub fn unbonder_public_key(&self) -> &PublicKey {
        &self.unbonder_public_key
    }

    /// Returns era which was used to create this unbonding request.
    pub fn era_of_creation(&self) -> EraId {
        self.era_of_creation
    }

    /// Returns unbonding amount.
    pub fn amount(&self) -> &U512 {
        &self.amount
    }
}

impl ToBytes for UnbondingPurse {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        result.extend(&self.bonding_purse.to_bytes()?);
        result.extend(&self.validator_public_key.to_bytes()?);
        result.extend(&self.unbonder_public_key.to_bytes()?);
        result.extend(&self.era_of_creation.to_bytes()?);
        result.extend(&self.amount.to_bytes()?);
        Ok(result)
    }
    fn serialized_length(&self) -> usize {
        self.bonding_purse.serialized_length()
            + self.validator_public_key.serialized_length()
            + self.unbonder_public_key.serialized_length()
            + self.era_of_creation.serialized_length()
            + self.amount.serialized_length()
    }
}

impl FromBytes for UnbondingPurse {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (bonding_purse, bytes) = FromBytes::from_bytes(bytes)?;
        let (validator_public_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (unbonder_public_key, bytes) = FromBytes::from_bytes(bytes)?;
        let (era_of_creation, bytes) = FromBytes::from_bytes(bytes)?;
        let (amount, bytes) = FromBytes::from_bytes(bytes)?;
        Ok((
            UnbondingPurse {
                bonding_purse,
                validator_public_key,
                unbonder_public_key,
                era_of_creation,
                amount,
            },
            bytes,
        ))
    }
}

impl CLTyped for UnbondingPurse {
    fn cl_type() -> CLType {
        CLType::Any
    }
}

#[cfg(any(feature = "gens", test))]
pub(crate) mod gens {
    use crate::{
        crypto::gens::public_key_arb,
        gens::{era_id_arb, u512_arb, uref_arb},
    };
    use proptest::prelude::*;

    use super::UnbondingPurse;

    prop_compose! {
        /// Creates an arbitrary [`UnbondingPurse`]
        pub fn unbonding_purse_arb()(bonding_purse in uref_arb(),
                                            validator_public_key in public_key_arb(),
                                            unbonder_public_key in public_key_arb(),
                                            era_of_creation in era_id_arb(),
                                            amount in u512_arb()) -> UnbondingPurse {
            UnbondingPurse {
                bonding_purse,
                validator_public_key,
                unbonder_public_key,
                era_of_creation,
                amount
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use once_cell::sync::Lazy;

    use crate::{
        bytesrepr,
        system::auction::{EraId, UnbondingPurse},
        AccessRights, PublicKey, SecretKey, URef, U512,
    };

    const BONDING_PURSE: URef = URef::new([41; 32], AccessRights::READ_ADD_WRITE);
    const ERA_OF_WITHDRAWAL: EraId = EraId::MAX;

    static VALIDATOR_PUBLIC_KEY: Lazy<PublicKey> = Lazy::new(|| {
        let secret_key = SecretKey::ed25519_from_bytes([42; SecretKey::ED25519_LENGTH]).unwrap();
        PublicKey::from(&secret_key)
    });
    static UNBONDER_PUBLIC_KEY: Lazy<PublicKey> = Lazy::new(|| {
        let secret_key = SecretKey::ed25519_from_bytes([43; SecretKey::ED25519_LENGTH]).unwrap();
        PublicKey::from(&secret_key)
    });
    static AMOUNT: Lazy<U512> = Lazy::new(|| U512::max_value() - 1);

    #[test]
    fn serialization_roundtrip() {
        let unbonding_purse = UnbondingPurse {
            bonding_purse: BONDING_PURSE,
            validator_public_key: VALIDATOR_PUBLIC_KEY.clone(),
            unbonder_public_key: UNBONDER_PUBLIC_KEY.clone(),
            era_of_creation: ERA_OF_WITHDRAWAL,
            amount: *AMOUNT,
        };

        bytesrepr::test_serialization_roundtrip(&unbonding_purse);
    }
    #[test]
    fn should_be_validator_condition() {
        let validator_unbonding_purse = UnbondingPurse::new(
            BONDING_PURSE,
            VALIDATOR_PUBLIC_KEY.clone(),
            VALIDATOR_PUBLIC_KEY.clone(),
            ERA_OF_WITHDRAWAL,
            *AMOUNT,
        );
        assert!(validator_unbonding_purse.is_validator());
    }

    #[test]
    fn should_be_delegator_condition() {
        let delegator_unbonding_purse = UnbondingPurse::new(
            BONDING_PURSE,
            VALIDATOR_PUBLIC_KEY.clone(),
            UNBONDER_PUBLIC_KEY.clone(),
            ERA_OF_WITHDRAWAL,
            *AMOUNT,
        );
        assert!(!delegator_unbonding_purse.is_validator());
    }
}
