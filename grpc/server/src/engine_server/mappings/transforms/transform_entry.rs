use std::convert::{TryFrom, TryInto};

use casper_execution_engine::shared::transform::Transform;
use casper_types::Key;

use crate::engine_server::{mappings::ParsingError, transforms::TransformEntry};

impl From<(Key, Transform)> for TransformEntry {
    fn from((key, transform): (Key, Transform)) -> Self {
        let mut pb_transform_entry = TransformEntry::new();
        pb_transform_entry.set_key(key.into());
        pb_transform_entry.set_transform(transform.into());
        pb_transform_entry
    }
}

impl TryFrom<TransformEntry> for (Key, Transform) {
    type Error = ParsingError;

    fn try_from(pb_transform_entry: TransformEntry) -> Result<Self, Self::Error> {
        let pb_key = pb_transform_entry
            .key
            .into_option()
            .ok_or_else(|| ParsingError::from("Protobuf TransformEntry missing Key field"))?;
        let key = pb_key.try_into()?;

        let pb_transform = pb_transform_entry
            .transform
            .into_option()
            .ok_or_else(|| ParsingError::from("Protobuf TransformEntry missing Transform field"))?;
        let transform = pb_transform.try_into()?;

        Ok((key, transform))
    }
}

#[cfg(test)]
mod tests {
    use proptest::proptest;

    use casper_execution_engine::shared::transform;
    use casper_types::gens;

    use super::*;
    use crate::engine_server::mappings::test_utils;

    proptest! {
        #[test]
        fn round_trip(
            key in gens::key_arb(),
            transform in transform::gens::transform_arb()
        ) {
            test_utils::protobuf_round_trip::<(Key, Transform), TransformEntry>((key, transform));
        }
    }
}
