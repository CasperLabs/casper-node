mod action_thresholds;
mod associated_keys;

use std::collections::BTreeSet;

use casper_types::{
    account::{
        AccountHash, ActionType, AddKeyFailure, RemoveKeyFailure, SetThresholdFailure,
        UpdateKeyFailure, Weight,
    },
    bytesrepr::{self, Error, FromBytes, ToBytes},
    contracts::NamedKeys,
    AccessRights, URef,
};

pub use action_thresholds::ActionThresholds;
pub use associated_keys::AssociatedKeys;

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Account {
    account_hash: AccountHash,
    named_keys: NamedKeys,
    main_purse: URef,
    associated_keys: AssociatedKeys,
    action_thresholds: ActionThresholds,
}

impl Account {
    pub fn new(
        account_hash: AccountHash,
        named_keys: NamedKeys,
        main_purse: URef,
        associated_keys: AssociatedKeys,
        action_thresholds: ActionThresholds,
    ) -> Self {
        Account {
            account_hash,
            named_keys,
            main_purse,
            associated_keys,
            action_thresholds,
        }
    }

    pub fn create(account: AccountHash, named_keys: NamedKeys, main_purse: URef) -> Self {
        let associated_keys = AssociatedKeys::new(account, Weight::new(1));
        let action_thresholds: ActionThresholds = Default::default();
        Account::new(
            account,
            named_keys,
            main_purse,
            associated_keys,
            action_thresholds,
        )
    }

    pub fn named_keys_append(&mut self, keys: &mut NamedKeys) {
        self.named_keys.append(keys);
    }

    pub fn named_keys(&self) -> &NamedKeys {
        &self.named_keys
    }

    pub fn named_keys_mut(&mut self) -> &mut NamedKeys {
        &mut self.named_keys
    }

    pub fn account_hash(&self) -> AccountHash {
        self.account_hash
    }

    pub fn main_purse(&self) -> URef {
        self.main_purse
    }

    /// Returns an [`AccessRights::ADD`]-only version of the [`URef`].
    pub fn main_purse_add_only(&self) -> URef {
        URef::new(self.main_purse.addr(), AccessRights::ADD)
    }

    pub fn associated_keys(&self) -> &AssociatedKeys {
        &self.associated_keys
    }

    pub fn action_thresholds(&self) -> &ActionThresholds {
        &self.action_thresholds
    }

    pub fn add_associated_key(
        &mut self,
        account_hash: AccountHash,
        weight: Weight,
    ) -> Result<(), AddKeyFailure> {
        self.associated_keys.add_key(account_hash, weight)
    }

    /// Checks if removing given key would properly satisfy thresholds.
    fn can_remove_key(&self, account_hash: AccountHash) -> bool {
        let total_weight_without = self
            .associated_keys
            .total_keys_weight_excluding(account_hash);

        // Returns true if the total weight calculated without given public key would be greater or
        // equal to all of the thresholds.
        total_weight_without >= *self.action_thresholds().deployment()
            && total_weight_without >= *self.action_thresholds().key_management()
    }

    /// Checks if adding a weight to a sum of all weights excluding the given key would make the
    /// resulting value to fall below any of the thresholds on account.
    fn can_update_key(&self, account_hash: AccountHash, weight: Weight) -> bool {
        // Calculates total weight of all keys excluding the given key
        let total_weight = self
            .associated_keys
            .total_keys_weight_excluding(account_hash);

        // Safely calculate new weight by adding the updated weight
        let new_weight = total_weight.value().saturating_add(weight.value());

        // Returns true if the new weight would be greater or equal to all of
        // the thresholds.
        new_weight >= self.action_thresholds().deployment().value()
            && new_weight >= self.action_thresholds().key_management().value()
    }

    pub fn remove_associated_key(
        &mut self,
        account_hash: AccountHash,
    ) -> Result<(), RemoveKeyFailure> {
        if self.associated_keys.contains_key(&account_hash) {
            // Check if removing this weight would fall below thresholds
            if !self.can_remove_key(account_hash) {
                return Err(RemoveKeyFailure::ThresholdViolation);
            }
        }
        self.associated_keys.remove_key(&account_hash)
    }

    pub fn update_associated_key(
        &mut self,
        account_hash: AccountHash,
        weight: Weight,
    ) -> Result<(), UpdateKeyFailure> {
        if let Some(current_weight) = self.associated_keys.get(&account_hash) {
            if weight < *current_weight {
                // New weight is smaller than current weight
                if !self.can_update_key(account_hash, weight) {
                    return Err(UpdateKeyFailure::ThresholdViolation);
                }
            }
        }
        self.associated_keys.update_key(account_hash, weight)
    }

    pub fn get_associated_key_weight(&self, account_hash: AccountHash) -> Option<&Weight> {
        self.associated_keys.get(&account_hash)
    }

    pub fn set_action_threshold(
        &mut self,
        action_type: ActionType,
        weight: Weight,
    ) -> Result<(), SetThresholdFailure> {
        // Verify if new threshold weight exceeds total weight of all associated
        // keys.
        self.can_set_threshold(weight)?;
        // Set new weight for given action
        self.action_thresholds.set_threshold(action_type, weight)
    }

    /// Verifies if user can set action threshold
    pub fn can_set_threshold(&self, new_threshold: Weight) -> Result<(), SetThresholdFailure> {
        let total_weight = self.associated_keys.total_keys_weight();
        if new_threshold > total_weight {
            return Err(SetThresholdFailure::InsufficientTotalWeight);
        }
        Ok(())
    }

    /// Checks whether all authorization keys are associated with this account
    pub fn can_authorize(&self, authorization_keys: &BTreeSet<AccountHash>) -> bool {
        !authorization_keys.is_empty()
            && authorization_keys
                .iter()
                .all(|e| self.associated_keys.contains_key(e))
    }

    /// Checks whether the sum of the weights of all authorization keys is
    /// greater or equal to deploy threshold.
    pub fn can_deploy_with(&self, authorization_keys: &BTreeSet<AccountHash>) -> bool {
        let total_weight = self
            .associated_keys
            .calculate_keys_weight(authorization_keys);

        total_weight >= *self.action_thresholds().deployment()
    }

    /// Checks whether the sum of the weights of all authorization keys is
    /// greater or equal to key management threshold.
    pub fn can_manage_keys_with(&self, authorization_keys: &BTreeSet<AccountHash>) -> bool {
        let total_weight = self
            .associated_keys
            .calculate_keys_weight(authorization_keys);

        total_weight >= *self.action_thresholds().key_management()
    }
}

impl ToBytes for Account {
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut result = bytesrepr::allocate_buffer(self)?;
        result.append(&mut self.account_hash.to_bytes()?);
        result.append(&mut self.named_keys.to_bytes()?);
        result.append(&mut self.main_purse.to_bytes()?);
        result.append(&mut self.associated_keys.to_bytes()?);
        result.append(&mut self.action_thresholds.to_bytes()?);
        Ok(result)
    }

    fn serialized_length(&self) -> usize {
        self.account_hash.serialized_length()
            + self.named_keys.serialized_length()
            + self.main_purse.serialized_length()
            + self.associated_keys.serialized_length()
            + self.action_thresholds.serialized_length()
    }
}

impl FromBytes for Account {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (account_hash, rem) = AccountHash::from_bytes(bytes)?;
        let (named_keys, rem) = NamedKeys::from_bytes(rem)?;
        let (main_purse, rem) = URef::from_bytes(rem)?;
        let (associated_keys, rem) = AssociatedKeys::from_bytes(rem)?;
        let (action_thresholds, rem) = ActionThresholds::from_bytes(rem)?;
        Ok((
            Account {
                account_hash,
                named_keys,
                main_purse,
                associated_keys,
                action_thresholds,
            },
            rem,
        ))
    }
}

#[cfg(any(feature = "gens", test))]
pub mod gens {
    use proptest::prelude::*;

    use casper_types::gens::{account_hash_arb, named_keys_arb, uref_arb};

    use super::*;
    use crate::shared::account::{
        action_thresholds::gens::action_thresholds_arb, associated_keys::gens::associated_keys_arb,
    };

    prop_compose! {
        pub fn account_arb()(
            account_hash in account_hash_arb(),
            urefs in named_keys_arb(3),
            purse in uref_arb(),
            thresholds in action_thresholds_arb(),
            mut associated_keys in associated_keys_arb(),
        ) -> Account {
                associated_keys.add_key(account_hash, Weight::new(1)).unwrap();
                Account::new(
                    account_hash,
                    urefs,
                    purse,
                    associated_keys,
                    thresholds,
                )
        }
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use casper_types::bytesrepr;

    use super::*;

    proptest! {
        #[test]
        fn test_value_account(acct in gens::account_arb()) {
            bytesrepr::test_serialization_roundtrip(&acct);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, iter::FromIterator};

    use casper_types::{
        account::{
            AccountHash, ActionType, RemoveKeyFailure, SetThresholdFailure, UpdateKeyFailure,
            Weight,
        },
        AccessRights, URef,
    };

    use super::*;

    #[test]
    fn associated_keys_can_authorize_keys() {
        let key_1 = AccountHash::new([0; 32]);
        let key_2 = AccountHash::new([1; 32]);
        let key_3 = AccountHash::new([2; 32]);
        let mut keys = AssociatedKeys::default();

        keys.add_key(key_2, Weight::new(2))
            .expect("should add key_1");
        keys.add_key(key_1, Weight::new(1))
            .expect("should add key_1");
        keys.add_key(key_3, Weight::new(3))
            .expect("should add key_1");

        let account = Account::new(
            AccountHash::new([0u8; 32]),
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            keys,
            // deploy: 33 (3*11)
            ActionThresholds::new(Weight::new(33), Weight::new(48))
                .expect("should create thresholds"),
        );

        assert!(account.can_authorize(&BTreeSet::from_iter(vec![key_3, key_2, key_1])));
        assert!(account.can_authorize(&BTreeSet::from_iter(vec![key_1, key_3, key_2])));

        assert!(account.can_authorize(&BTreeSet::from_iter(vec![key_1, key_2])));
        assert!(account.can_authorize(&BTreeSet::from_iter(vec![key_1])));

        assert!(!account.can_authorize(&BTreeSet::from_iter(vec![
            key_1,
            key_2,
            AccountHash::new([42; 32])
        ])));
        assert!(!account.can_authorize(&BTreeSet::from_iter(vec![
            AccountHash::new([42; 32]),
            key_1,
            key_2
        ])));
        assert!(!account.can_authorize(&BTreeSet::from_iter(vec![
            AccountHash::new([43; 32]),
            AccountHash::new([44; 32]),
            AccountHash::new([42; 32])
        ])));
        assert!(!account.can_authorize(&BTreeSet::new()));
    }

    #[test]
    fn account_can_deploy_with() {
        let associated_keys = {
            let mut res = AssociatedKeys::new(AccountHash::new([1u8; 32]), Weight::new(1));
            res.add_key(AccountHash::new([2u8; 32]), Weight::new(11))
                .expect("should add key 1");
            res.add_key(AccountHash::new([3u8; 32]), Weight::new(11))
                .expect("should add key 2");
            res.add_key(AccountHash::new([4u8; 32]), Weight::new(11))
                .expect("should add key 3");
            res
        };
        let account = Account::new(
            AccountHash::new([0u8; 32]),
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            // deploy: 33 (3*11)
            ActionThresholds::new(Weight::new(33), Weight::new(48))
                .expect("should create thresholds"),
        );

        // sum: 22, required 33 - can't deploy
        assert!(!account.can_deploy_with(&BTreeSet::from_iter(vec![
            AccountHash::new([3u8; 32]),
            AccountHash::new([2u8; 32]),
        ])));

        // sum: 33, required 33 - can deploy
        assert!(account.can_deploy_with(&BTreeSet::from_iter(vec![
            AccountHash::new([4u8; 32]),
            AccountHash::new([3u8; 32]),
            AccountHash::new([2u8; 32]),
        ])));

        // sum: 34, required 33 - can deploy
        assert!(account.can_deploy_with(&BTreeSet::from_iter(vec![
            AccountHash::new([2u8; 32]),
            AccountHash::new([1u8; 32]),
            AccountHash::new([4u8; 32]),
            AccountHash::new([3u8; 32]),
        ])));
    }

    #[test]
    fn account_can_manage_keys_with() {
        let associated_keys = {
            let mut res = AssociatedKeys::new(AccountHash::new([1u8; 32]), Weight::new(1));
            res.add_key(AccountHash::new([2u8; 32]), Weight::new(11))
                .expect("should add key 1");
            res.add_key(AccountHash::new([3u8; 32]), Weight::new(11))
                .expect("should add key 2");
            res.add_key(AccountHash::new([4u8; 32]), Weight::new(11))
                .expect("should add key 3");
            res
        };
        let account = Account::new(
            AccountHash::new([0u8; 32]),
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            // deploy: 33 (3*11)
            ActionThresholds::new(Weight::new(11), Weight::new(33))
                .expect("should create thresholds"),
        );

        // sum: 22, required 33 - can't manage
        assert!(!account.can_manage_keys_with(&BTreeSet::from_iter(vec![
            AccountHash::new([3u8; 32]),
            AccountHash::new([2u8; 32]),
        ])));

        // sum: 33, required 33 - can manage
        assert!(account.can_manage_keys_with(&BTreeSet::from_iter(vec![
            AccountHash::new([4u8; 32]),
            AccountHash::new([3u8; 32]),
            AccountHash::new([2u8; 32]),
        ])));

        // sum: 34, required 33 - can manage
        assert!(account.can_manage_keys_with(&BTreeSet::from_iter(vec![
            AccountHash::new([2u8; 32]),
            AccountHash::new([1u8; 32]),
            AccountHash::new([4u8; 32]),
            AccountHash::new([3u8; 32]),
        ])));
    }

    #[test]
    fn set_action_threshold_higher_than_total_weight() {
        let identity_key = AccountHash::new([1u8; 32]);
        let key_1 = AccountHash::new([2u8; 32]);
        let key_2 = AccountHash::new([3u8; 32]);
        let key_3 = AccountHash::new([4u8; 32]);
        let associated_keys = {
            let mut res = AssociatedKeys::new(identity_key, Weight::new(1));
            res.add_key(key_1, Weight::new(2))
                .expect("should add key 1");
            res.add_key(key_2, Weight::new(3))
                .expect("should add key 2");
            res.add_key(key_3, Weight::new(4))
                .expect("should add key 3");
            res
        };
        let mut account = Account::new(
            AccountHash::new([0u8; 32]),
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            // deploy: 33 (3*11)
            ActionThresholds::new(Weight::new(33), Weight::new(48))
                .expect("should create thresholds"),
        );

        assert_eq!(
            account
                .set_action_threshold(ActionType::Deployment, Weight::new(1 + 2 + 3 + 4 + 1))
                .unwrap_err(),
            SetThresholdFailure::InsufficientTotalWeight,
        );
        assert_eq!(
            account
                .set_action_threshold(ActionType::Deployment, Weight::new(1 + 2 + 3 + 4 + 245))
                .unwrap_err(),
            SetThresholdFailure::InsufficientTotalWeight,
        )
    }

    #[test]
    fn remove_key_would_violate_action_thresholds() {
        let identity_key = AccountHash::new([1u8; 32]);
        let key_1 = AccountHash::new([2u8; 32]);
        let key_2 = AccountHash::new([3u8; 32]);
        let key_3 = AccountHash::new([4u8; 32]);
        let associated_keys = {
            let mut res = AssociatedKeys::new(identity_key, Weight::new(1));
            res.add_key(key_1, Weight::new(2))
                .expect("should add key 1");
            res.add_key(key_2, Weight::new(3))
                .expect("should add key 2");
            res.add_key(key_3, Weight::new(4))
                .expect("should add key 3");
            res
        };
        let mut account = Account::new(
            AccountHash::new([0u8; 32]),
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            // deploy: 33 (3*11)
            ActionThresholds::new(Weight::new(1 + 2 + 3 + 4), Weight::new(1 + 2 + 3 + 4 + 5))
                .expect("should create thresholds"),
        );

        assert_eq!(
            account.remove_associated_key(key_3).unwrap_err(),
            RemoveKeyFailure::ThresholdViolation,
        )
    }

    #[test]
    fn updating_key_would_violate_action_thresholds() {
        let identity_key = AccountHash::new([1u8; 32]);
        let identity_key_weight = Weight::new(1);
        let key_1 = AccountHash::new([2u8; 32]);
        let key_1_weight = Weight::new(2);
        let key_2 = AccountHash::new([3u8; 32]);
        let key_2_weight = Weight::new(3);
        let key_3 = AccountHash::new([4u8; 32]);
        let key_3_weight = Weight::new(4);
        let associated_keys = {
            let mut res = AssociatedKeys::new(identity_key, identity_key_weight);
            res.add_key(key_1, key_1_weight).expect("should add key 1");
            res.add_key(key_2, key_2_weight).expect("should add key 2");
            res.add_key(key_3, key_3_weight).expect("should add key 3");
            // 1 + 2 + 3 + 4
            res
        };

        let deployment_threshold = Weight::new(
            identity_key_weight.value()
                + key_1_weight.value()
                + key_2_weight.value()
                + key_3_weight.value(),
        );
        let key_management_threshold = Weight::new(deployment_threshold.value() + 1);
        let mut account = Account::new(
            identity_key,
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            // deploy: 33 (3*11)
            ActionThresholds::new(deployment_threshold, key_management_threshold)
                .expect("should create thresholds"),
        );

        // Decreases by 3
        assert_eq!(
            account
                .clone()
                .update_associated_key(key_3, Weight::new(1))
                .unwrap_err(),
            UpdateKeyFailure::ThresholdViolation,
        );

        // increase total weight (12)
        account
            .update_associated_key(identity_key, Weight::new(3))
            .unwrap();

        // variant a) decrease total weight by 1 (total 11)
        account
            .clone()
            .update_associated_key(key_3, Weight::new(3))
            .unwrap();
        // variant b) decrease total weight by 3 (total 9) - fail
        assert_eq!(
            account
                .update_associated_key(key_3, Weight::new(1))
                .unwrap_err(),
            UpdateKeyFailure::ThresholdViolation
        );
    }

    #[test]
    fn overflowing_should_allow_removal() {
        let identity_key = AccountHash::new([42; 32]);
        let key_1 = AccountHash::new([2u8; 32]);
        let key_2 = AccountHash::new([3u8; 32]);

        let associated_keys = {
            // Identity
            let mut res = AssociatedKeys::new(identity_key, Weight::new(1));

            // Spare key
            res.add_key(key_1, Weight::new(2))
                .expect("should add key 1");
            // Big key
            res.add_key(key_2, Weight::new(255))
                .expect("should add key 2");

            res
        };

        let mut account = Account::new(
            identity_key,
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            ActionThresholds::new(Weight::new(1), Weight::new(254))
                .expect("should create thresholds"),
        );

        account.remove_associated_key(key_1).expect("should work")
    }

    #[test]
    fn overflowing_should_allow_updating() {
        let identity_key = AccountHash::new([1; 32]);
        let identity_key_weight = Weight::new(1);
        let key_1 = AccountHash::new([2u8; 32]);
        let key_1_weight = Weight::new(3);
        let key_2 = AccountHash::new([3u8; 32]);
        let key_2_weight = Weight::new(255);
        let deployment_threshold = Weight::new(1);
        let key_management_threshold = Weight::new(254);

        let associated_keys = {
            // Identity
            let mut res = AssociatedKeys::new(identity_key, identity_key_weight);

            // Spare key
            res.add_key(key_1, key_1_weight).expect("should add key 1");
            // Big key
            res.add_key(key_2, key_2_weight).expect("should add key 2");

            res
        };

        let mut account = Account::new(
            identity_key,
            NamedKeys::new(),
            URef::new([0u8; 32], AccessRights::READ_ADD_WRITE),
            associated_keys,
            ActionThresholds::new(deployment_threshold, key_management_threshold)
                .expect("should create thresholds"),
        );

        // decrease so total weight would be changed from 1 + 3 + 255 to 1 + 1 + 255
        account
            .update_associated_key(key_1, Weight::new(1))
            .expect("should work");
    }
}
