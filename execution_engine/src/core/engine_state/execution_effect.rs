use casper_types::Key;

use super::op::Op;
use crate::shared::{additive_map::AdditiveMap, transform::Transform};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionEffect {
    pub ops: AdditiveMap<Key, Op>,
    pub transforms: AdditiveMap<Key, Transform>,
}

impl ExecutionEffect {
    pub fn new(ops: AdditiveMap<Key, Op>, transforms: AdditiveMap<Key, Transform>) -> Self {
        ExecutionEffect { ops, transforms }
    }
}

impl From<&ExecutionEffect> for casper_types::ExecutionEffect {
    fn from(effect: &ExecutionEffect) -> Self {
        casper_types::ExecutionEffect {
            operations: effect
                .ops
                .iter()
                .map(|(key, op)| casper_types::Operation::new(*key, op.into()))
                .collect(),
            transforms: effect
                .transforms
                .iter()
                .map(|(key, transform)| casper_types::TransformEntry::new(*key, transform.into()))
                .collect(),
        }
    }
}
