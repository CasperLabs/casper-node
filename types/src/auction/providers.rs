
use crate::{
    account::AccountHash,
    bytesrepr::{FromBytes, ToBytes},
    system_contract_errors::auction::Error,
    CLTyped, Key, PublicKey, URef, U512,
};

/// Provider of runtime host functionality.
pub trait RuntimeProvider {
    /// This method should return the caller of the current context.
    fn get_caller(&self) -> AccountHash;
}

/// Provides functionality of a contract storage.
pub trait StorageProvider {
    /// Compatible error type.
    type Error: From<Error>;

    /// Obtain named [`Key`].
    fn get_key(&mut self, name: &str) -> Option<Key>;

    /// Read data from [`URef`].
    fn read<T: FromBytes + CLTyped>(&mut self, uref: URef) -> Result<Option<T>, Self::Error>;

    /// Write data to [`URef].
    fn write<T: ToBytes + CLTyped>(&mut self, uref: URef, value: T) -> Result<(), Self::Error>;
}

/// Provides functionality of a mint contract.
pub trait MintProvider {
    /// Error representation for Mint errors.
    type Error: From<Error>;

    /// Bond specified amount from a purse.
    fn bond(
        &mut self,
        public_key: PublicKey,
        amount: U512,
        purse: URef,
    ) -> Result<(URef, U512), Self::Error>;

    /// Unbond specified amount.
    fn unbond(&mut self, public_key: PublicKey, amount: U512) -> Result<(URef, U512), Self::Error>;
}

/// Provides functionality of a system module.
pub trait SystemProvider {
    /// Error representation for system errors.
    type Error: From<Error>;

    /// Creates new purse.
    fn create_purse(&mut self) -> URef;

    /// Gets purse balance.
    fn get_balance(&mut self, purse: URef) -> Result<Option<U512>, Self::Error>;

    /// Transfers specified `amount` of tokens from `source` purse into a `target` purse.
    fn transfer_from_purse_to_purse(
        &mut self,
        source: URef,
        target: URef,
        amount: U512,
    ) -> Result<(), Self::Error>;
}

// /// Provides data from storage.
// pub trait DataProvider {
//     /// Error representation for data provider errors.
//     type Error: From<Error>;

//     /// Gets delegation map
//     fn get_delegations_map(&mut self) -> Result<DelegationsMap, Self::Error>;

//     /// Sets delegation map
//     fn set_delegations_map(&mut self, delegations_map: DelegationsMap) -> Result<(), Self::Error>;

//     /// Gets tally map
//     fn get_tally_map(&mut self) -> Result<TallyMap, Self::Error>;

//     /// Sets tally map
//     fn set_tally_map(&mut self, tally_map: TallyMap) -> Result<(), Self::Error>;

//     /// Gets reward per stake map
//     fn get_reward_per_stake_map(&mut self) -> Result<RewardPerStakeMap, Self::Error>;

//     /// Sets reward per stake map
//     fn set_reward_per_stake_map(
//         &mut self,
//         reward_per_stake_map: RewardPerStakeMap,
//     ) -> Result<(), Self::Error>;

//     /// Gets total delegator stake map
//     fn get_total_delegator_stake_map(&mut self) -> Result<TotalDelegatorStakeMap, Self::Error>;

//     /// Sets total delegator stake map
//     fn set_total_delegator_stake_map(
//         &mut self,
//         total_delegator_stake_map: TotalDelegatorStakeMap,
//     ) -> Result<(), Self::Error>;

//     /// Gets delegator reward pool map
//     fn get_delegator_reward_pool_map(&mut self) -> Result<DelegatorRewardPoolMap, Self::Error>;

//     /// Sets delegator reward pool map
//     fn set_delegator_reward_pool_map(
//         &mut self,
//         delegator_reward_pool_map: DelegatorRewardPoolMap,
//     ) -> Result<(), Self::Error>;
// }

// /// Provides data from storage.
// pub trait DelegationProvider {
//     /// Error representation for data provider errors.
//     type Error: From<Error>;

//     /// Adds a new delegator to delegators, or tops off a current
//     /// one. 
//     fn delegate(
//         &mut self,
//         delegator_account_hash: AccountHash,
//         source_purse: URef,
//         validator_account_hash: AccountHash,
//         delegation_amount: U512,
//     ) -> Result<(URef, U512), Self::Error>;

//     /// Removes a quantity (or the entry altogether, if the
//     /// remaining quantity is 0) of motes from the entry in delegators
//     /// and calls unbond in the Mint contract to create a new unbonding
//     /// purse.
//     fn undelegate(
//         &mut self,
//         delegator_account_hash: AccountHash,
//         validator_account_hash: AccountHash,
//         quantity: U512,
//     ) -> Result<U512, Self::Error>;

//     /// Distributes rewards to the delegators associated with `validator_account_hash`.
//     fn distribute_to_delegators(
//         &mut self,
//         validator_account_hash: AccountHash,
//         purse: URef,
//     ) -> Result<(), Self::Error>;

//     /// Returns the total rewards a delegator has earned from delegating to a specific validator.
//     fn delegation_reward(
//         &mut self,
//         validator_account_hash: AccountHash,
//         delegator_account_hash: AccountHash,
//     ) -> Result<U512, Self::Error>;

//     /// Pays out the entire accumulated delegation rewards to the destination purse.
//     fn withdraw_reward(
//         &mut self,
//         validator_account_hash: AccountHash,
//         delegator_account_hash: AccountHash,
//         purse: URef,
//     ) -> Result<U512, Self::Error>;
// }