use std::collections::VecDeque;

use casper_types::bytesrepr::{self, Bytes, FromBytes, ToBytes};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    shared::newtypes::Blake2bHash,
    storage::trie::{Pointer, Trie, RADIX},
};

const TRIE_MERKLE_PROOF_STEP_NODE_ID: u8 = 0;
const TRIE_MERKLE_PROOF_STEP_EXTENSION_ID: u8 = 1;

/// A component of a proof that an entry exists in the Merkle trie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TrieMerkleProofStep {
    /// Corresponds to [`Trie::Node`]
    Node {
        hole_index: u8,
        indexed_pointers_with_hole: Vec<(u8, Pointer)>,
    },
    /// Corresponds to [`Trie::Extension`]
    #[schemars(with = "String", description = "Hex encoded extension")]
    Extension { affix: Bytes },
}

impl TrieMerkleProofStep {
    /// Constructor for  [`TrieMerkleProofStep::Node`]
    pub fn node(hole_index: u8, indexed_pointers_with_hole: Vec<(u8, Pointer)>) -> Self {
        Self::Node {
            hole_index,
            indexed_pointers_with_hole,
        }
    }

    /// Constructor for  [`TrieMerkleProofStep::Extension`]
    pub fn extension(affix: Vec<u8>) -> Self {
        Self::Extension {
            affix: affix.into(),
        }
    }
}

impl ToBytes for TrieMerkleProofStep {
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut ret: Vec<u8> = bytesrepr::allocate_buffer(self)?;
        match self {
            TrieMerkleProofStep::Node {
                hole_index,
                indexed_pointers_with_hole,
            } => {
                ret.push(TRIE_MERKLE_PROOF_STEP_NODE_ID);
                ret.push(*hole_index);
                ret.append(&mut indexed_pointers_with_hole.to_bytes()?)
            }
            TrieMerkleProofStep::Extension { affix } => {
                ret.push(TRIE_MERKLE_PROOF_STEP_EXTENSION_ID);
                ret.append(&mut affix.to_bytes()?)
            }
        };
        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        std::mem::size_of::<u8>()
            + match self {
                TrieMerkleProofStep::Node {
                    hole_index,
                    indexed_pointers_with_hole,
                } => {
                    (*hole_index).serialized_length()
                        + (*indexed_pointers_with_hole).serialized_length()
                }
                TrieMerkleProofStep::Extension { affix } => affix.serialized_length(),
            }
    }
}

impl FromBytes for TrieMerkleProofStep {
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (tag, rem): (u8, &[u8]) = FromBytes::from_bytes(bytes)?;
        match tag {
            TRIE_MERKLE_PROOF_STEP_NODE_ID => {
                let (hole_index, rem): (u8, &[u8]) = FromBytes::from_bytes(rem)?;
                let (indexed_pointers_with_hole, rem): (Vec<(u8, Pointer)>, &[u8]) =
                    FromBytes::from_bytes(rem)?;
                Ok((
                    TrieMerkleProofStep::Node {
                        hole_index,
                        indexed_pointers_with_hole,
                    },
                    rem,
                ))
            }
            TRIE_MERKLE_PROOF_STEP_EXTENSION_ID => {
                let (affix, rem): (_, &[u8]) = FromBytes::from_bytes(rem)?;
                Ok((TrieMerkleProofStep::Extension { affix }, rem))
            }
            _ => Err(bytesrepr::Error::Formatting),
        }
    }
}

/// A proof that a node with a specified `key` and `value` is present in the Merkle trie.
/// Given a state hash `x`, one can validate a proof `p` by checking `x == p.compute_state_hash()`.
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema, Serialize, Deserialize)]
pub struct TrieMerkleProof<K, V> {
    key: K,
    value: V,
    proof_steps: VecDeque<TrieMerkleProofStep>,
}

impl<K, V> TrieMerkleProof<K, V> {
    /// Constructor for [`TrieMerkleProof`]
    pub fn new(key: K, value: V, proof_steps: VecDeque<TrieMerkleProofStep>) -> Self {
        TrieMerkleProof {
            key,
            value,
            proof_steps,
        }
    }

    /// Getter for the key in [`TrieMerkleProof`]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Getter for the value in [`TrieMerkleProof`]
    pub fn value(&self) -> &V {
        &self.value
    }

    /// Getter for the proof steps in [`TrieMerkleProof`]
    pub fn proof_steps(&self) -> &VecDeque<TrieMerkleProofStep> {
        &self.proof_steps
    }

    /// Transforms a [`TrieMerkleProof`] into the value it contains
    pub fn into_value(self) -> V {
        self.value
    }
}

impl<K, V> TrieMerkleProof<K, V>
where
    K: ToBytes + Copy + Clone,
    V: ToBytes + Clone,
{
    /// Recomputes a state root hash from a [`TrieMerkleProof`].
    /// This is done in the following steps:
    ///
    /// 1. Using [`TrieMerkleProof::key`] and [`TrieMerkleProof::value`], construct a
    /// [`Trie::Leaf`] and compute a hash for that leaf.
    ///
    /// 2. We then iterate over [`TrieMerkleProof::proof_steps`] left to right, using the hash from
    /// the previous step combined with the next step to compute a new hash.
    ///
    /// 3. When there are no more steps, we return the final hash we have computed.
    ///
    /// The steps in this function reflect `operations::rehash`.
    pub fn compute_state_hash(&self) -> Result<Blake2bHash, bytesrepr::Error> {
        let mut hash = {
            let leaf_bytes = Trie::leaf(self.key, self.value.to_owned()).to_bytes()?;
            Blake2bHash::new(&leaf_bytes)
        };

        for (proof_step_index, proof_step) in self.proof_steps.iter().enumerate() {
            let pointer = if proof_step_index == 0 {
                Pointer::LeafPointer(hash)
            } else {
                Pointer::NodePointer(hash)
            };
            let proof_step_bytes = match proof_step {
                TrieMerkleProofStep::Node {
                    hole_index,
                    indexed_pointers_with_hole,
                } => {
                    let hole_index = *hole_index;
                    assert!(hole_index as usize <= RADIX, "hole_index exceeded RADIX");
                    let mut indexed_pointers = indexed_pointers_with_hole.to_owned();
                    indexed_pointers.push((hole_index, pointer));
                    Trie::<K, V>::node(&indexed_pointers).to_bytes()?
                }
                TrieMerkleProofStep::Extension { affix } => {
                    Trie::<K, V>::extension(affix.clone().into(), pointer).to_bytes()?
                }
            };
            hash = Blake2bHash::new(&proof_step_bytes);
        }
        Ok(hash)
    }
}

impl<K, V> ToBytes for TrieMerkleProof<K, V>
where
    K: ToBytes,
    V: ToBytes,
{
    fn to_bytes(&self) -> Result<Vec<u8>, bytesrepr::Error> {
        let mut ret: Vec<u8> = bytesrepr::allocate_buffer(self)?;
        ret.append(&mut self.key.to_bytes()?);
        ret.append(&mut self.value.to_bytes()?);
        ret.append(&mut self.proof_steps.to_bytes()?);
        Ok(ret)
    }

    fn serialized_length(&self) -> usize {
        self.key.serialized_length()
            + self.value.serialized_length()
            + self.proof_steps.serialized_length()
    }
}

impl<K, V> FromBytes for TrieMerkleProof<K, V>
where
    K: FromBytes,
    V: FromBytes,
{
    fn from_bytes(bytes: &[u8]) -> Result<(Self, &[u8]), bytesrepr::Error> {
        let (key, rem): (K, &[u8]) = FromBytes::from_bytes(bytes)?;
        let (value, rem): (V, &[u8]) = FromBytes::from_bytes(rem)?;
        let (proof_steps, rem): (VecDeque<TrieMerkleProofStep>, &[u8]) =
            FromBytes::from_bytes(rem)?;
        Ok((
            TrieMerkleProof {
                key,
                value,
                proof_steps,
            },
            rem,
        ))
    }
}

#[cfg(test)]
mod gens {
    use proptest::{collection::vec, prelude::*};

    use casper_types::{gens::key_arb, Key};

    use crate::{
        shared::stored_value::{gens::stored_value_arb, StoredValue},
        storage::trie::{
            gens::trie_pointer_arb,
            merkle_proof::{TrieMerkleProof, TrieMerkleProofStep},
            RADIX,
        },
    };

    const POINTERS_SIZE: usize = RADIX / 8;
    const AFFIX_SIZE: usize = 6;
    const STEPS_SIZE: usize = 6;

    pub fn trie_merkle_proof_step_arb() -> impl Strategy<Value = TrieMerkleProofStep> {
        prop_oneof![
            (
                <u8>::arbitrary(),
                vec((<u8>::arbitrary(), trie_pointer_arb()), POINTERS_SIZE)
            )
                .prop_map(|(hole_index, indexed_pointers_with_hole)| {
                    TrieMerkleProofStep::Node {
                        hole_index,
                        indexed_pointers_with_hole,
                    }
                }),
            vec(<u8>::arbitrary(), AFFIX_SIZE).prop_map(|affix| {
                TrieMerkleProofStep::Extension {
                    affix: affix.into(),
                }
            })
        ]
    }

    pub fn trie_merkle_proof_arb() -> impl Strategy<Value = TrieMerkleProof<Key, StoredValue>> {
        (
            key_arb(),
            stored_value_arb(),
            vec(trie_merkle_proof_step_arb(), STEPS_SIZE),
        )
            .prop_map(|(key, value, proof_steps)| {
                TrieMerkleProof::new(key, value, proof_steps.into())
            })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use casper_types::bytesrepr;

    use super::gens;

    proptest! {
        #[test]
        fn trie_merkle_proof_step_serialization_is_correct(
            step in gens::trie_merkle_proof_step_arb()
        ) {
            bytesrepr::test_serialization_roundtrip(&step)
        }

        #[test]
        fn trie_merkle_proof_serialization_is_correct(
            proof in gens::trie_merkle_proof_arb()
        ) {
            bytesrepr::test_serialization_roundtrip(&proof)
        }
    }
}
