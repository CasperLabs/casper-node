use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    crypto::{
        asymmetric_key::{PublicKey, SecretKey},
        hash::Digest,
    },
    rpcs::docs::DocExample,
    types::json_compatibility,
};
use casper_types::{
    auction::{Bid as AuctionBid, Bids as AuctionBids, EraValidators as AuctionEraValidators},
    bytesrepr::{FromBytes, ToBytes},
    U512,
};

/// Bids table.
pub type Bids = BTreeMap<json_compatibility::PublicKey, Bid>;
/// Validator weights by validator key.
pub type ValidatorWeights = BTreeMap<json_compatibility::PublicKey, U512>;
/// List of era validators
pub type EraValidators = BTreeMap<u64, ValidatorWeights>;

/// An entry in a founding validator map.
#[derive(PartialEq, Debug, Deserialize, Serialize, Clone)]
pub struct Bid {
    /// The purse that was used for bonding.
    pub bonding_purse: String,
    /// The total amount of staked tokens.
    pub staked_amount: U512,
    /// Delegation rate.
    pub delegation_rate: u64,
    /// A flag that represents a winning entry.
    ///
    /// `Some` indicates locked funds for a specific era and an autowin status, and `None` case
    /// means that funds are unlocked and autowin status is removed.
    pub release_era: Option<u64>,
}

impl From<AuctionBid> for Bid {
    fn from(bid: AuctionBid) -> Self {
        Bid {
            bonding_purse: bid.bonding_purse().to_formatted_string(),
            staked_amount: *bid.staked_amount(),
            delegation_rate: *bid.delegation_rate(),
            release_era: bid.release_era(),
        }
    }
}

/// Data structure summarizing auction contract data.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct AuctionState {
    /// Global state hash
    pub state_root_hash: Digest,
    /// Block height
    pub block_height: u64,
    /// Era validators
    pub era_validators: Option<EraValidators>,
    /// All bids.
    bids: Option<Bids>,
}

impl AuctionState {
    /// Create new instance of `AuctionState`
    pub fn new(
        state_root_hash: Digest,
        block_height: u64,
        bids: Option<AuctionBids>,
        era_validators: Option<AuctionEraValidators>,
    ) -> Self {
        let bids = bids.map(|items| {
            items
                .into_iter()
                .map(|(public_key, bid)| (public_key.into(), bid.into()))
                .collect()
        });

        let era_validators = era_validators.map(|items| {
            items
                .into_iter()
                .map(|(era_id, validator_weights)| {
                    (
                        era_id,
                        validator_weights
                            .into_iter()
                            .map(|(public_key, weight)| (public_key.into(), weight))
                            .collect(),
                    )
                })
                .collect()
        });

        AuctionState {
            state_root_hash,
            block_height,
            bids,
            era_validators,
        }
    }
}

lazy_static! {
    static ref ERA_VALIDATORS: EraValidators = {
        let secret_key_1 = SecretKey::doc_example();
        let public_key_1 = PublicKey::from(secret_key_1);
        let asm_bytes = public_key_1.to_bytes().unwrap();
        let (casper_key, _) = casper_types::PublicKey::from_bytes(&asm_bytes).unwrap();
        let json_key = json_compatibility::PublicKey::from(casper_key);

        let mut validator_weights = BTreeMap::new();
        validator_weights.insert(json_key, U512::from(10));

        let mut era_validators = BTreeMap::new();
        era_validators.insert(10, validator_weights);

        era_validators
    };
    static ref BIDS: Bids = {
        let bonding_purse = String::from(
            "uref-09480c3248ef76b603d386f3f4f8a5f87f597d4eaffd475433f861af187ab5db-007",
        );
        let staked_amount = U512::from(10);
        let delegation_rate = 10;
        let release_era = Some(43);

        let bid = Bid {
            bonding_purse,
            staked_amount,
            delegation_rate,
            release_era,
        };

        let secret_key_1 = SecretKey::doc_example();
        let public_key_1 = PublicKey::from(secret_key_1);
        let asm_bytes = public_key_1.to_bytes().unwrap();
        let (casper_key, _) = casper_types::PublicKey::from_bytes(&asm_bytes).unwrap();
        let json_key = json_compatibility::PublicKey::from(casper_key);

        let mut bids = BTreeMap::new();
        bids.insert(json_key, bid);

        bids
    };
    static ref AUCTION_INFO: AuctionState = {
        let state_root_hash = Digest::doc_example();
        let height: u64 = 10;
        let era_validators = Some(EraValidators::doc_example().clone());
        let bids = Some(Bids::doc_example().clone());
        AuctionState {
            state_root_hash,
            block_height: height,
            era_validators,
            bids,
        }
    };
}

impl DocExample for AuctionState {
    fn doc_example() -> &'static Self {
        &*AUCTION_INFO
    }
}

impl DocExample for EraValidators {
    fn doc_example() -> &'static Self {
        &*ERA_VALIDATORS
    }
}

impl DocExample for Bids {
    fn doc_example() -> &'static Self {
        &*BIDS
    }
}
