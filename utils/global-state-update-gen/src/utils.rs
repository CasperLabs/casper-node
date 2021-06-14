use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryInto,
};

use casper_execution_engine::shared::{newtypes::Blake2bHash, stored_value::StoredValue};
use casper_types::{
    bytesrepr::ToBytes, check_summed_hex, system::auction::SeigniorageRecipientsSnapshot, Key,
    PublicKey,
};

/// Parses a Blake2bHash from a string. Panics if parsing fails.
pub fn hash_from_str(hex_str: &str) -> Blake2bHash {
    (&check_summed_hex::decode(hex_str).unwrap()[..])
        .try_into()
        .unwrap()
}

/// Prints a global state update entry in a format ready for inclusion in a TOML file.
pub(crate) fn print_entry(key: &Key, value: &StoredValue) {
    println!("[[entries]]");
    println!("key = \"{}\"", key.to_formatted_string());
    println!("value = \"{}\"", base64::encode(value.to_bytes().unwrap()));
    println!();
}

pub struct ValidatorsDiff {
    pub added: BTreeSet<PublicKey>,
    pub removed: BTreeSet<PublicKey>,
}

/// Calculates the sets of added and removed validators between the two snapshots.
pub fn validators_diff(
    old_snapshot: &SeigniorageRecipientsSnapshot,
    new_snapshot: &SeigniorageRecipientsSnapshot,
) -> ValidatorsDiff {
    let old_validators: BTreeSet<_> = old_snapshot
        .values()
        .flat_map(BTreeMap::keys)
        .cloned()
        .collect();
    let new_validators: BTreeSet<_> = new_snapshot
        .values()
        .flat_map(BTreeMap::keys)
        .cloned()
        .collect();

    ValidatorsDiff {
        added: new_validators
            .difference(&old_validators)
            .cloned()
            .collect(),
        removed: old_validators
            .difference(&new_validators)
            .cloned()
            .collect(),
    }
}
