use casper_engine_test_support::internal::UpgradeRequestBuilder;
use casper_types::{EraId, ProtocolVersion};

use crate::lmdb_fixture;

const DEFAULT_ACTIVATION_POINT: EraId = EraId::new(1);

#[ignore]
#[test]
fn should_migrate_protocol_data_after_major_version_bump_from_1_2_0() {
    let (mut builder, lmdb_fixture_state, _temp_dir) =
        lmdb_fixture::builder_from_global_state_fixture(lmdb_fixture::RELEASE_1_2_0);

    let current_protocol_version = serde_json::from_value(
        lmdb_fixture_state
            .genesis_request
            .get("protocol_version")
            .cloned()
            .unwrap(),
    )
    .unwrap();

    let legacy_protocol_data = builder
        .get_engine_state()
        .get_protocol_data(current_protocol_version)
        .expect("should have result")
        .expect("should have protocol data");

    let protocol_version_v1_3_0 = ProtocolVersion::from_parts(
        current_protocol_version.value().major,
        current_protocol_version.value().minor + 1,
        0,
    );

    // Upgrade 1.2.0 -> 1.3.0 should read legacy protocol data format, and write new protocol data
    // format.
    let mut upgrade_request_v1_3_0 = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(current_protocol_version)
            .with_new_protocol_version(protocol_version_v1_3_0)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request_v1_3_0)
        .expect_upgrade_success();

    let protocol_data_v1_3_0 = builder
        .get_engine_state()
        .get_protocol_data(protocol_version_v1_3_0)
        .expect("should have result")
        .expect("should have protocol data");

    let protocol_version_v1_4_0 = ProtocolVersion::from_parts(
        protocol_version_v1_3_0.value().major,
        protocol_version_v1_3_0.value().minor + 1,
        0,
    );

    // Upgrade 1.3.0 -> 1.4.0 should read new protocol data format and write new protocol data
    // format
    let mut upgrade_request_v1_4_0 = {
        UpgradeRequestBuilder::new()
            .with_current_protocol_version(protocol_version_v1_3_0)
            .with_new_protocol_version(protocol_version_v1_4_0)
            .with_activation_point(DEFAULT_ACTIVATION_POINT)
            .build()
    };

    builder
        .upgrade_with_upgrade_request(&mut upgrade_request_v1_4_0)
        .expect_upgrade_success();

    let protocol_data_v1_4_0 = builder
        .get_engine_state()
        .get_protocol_data(protocol_version_v1_4_0)
        .expect("should have result")
        .expect("should have protocol data");

    // NOTE: Those assertions are written as is to fail intentionally once `ProtocolData` object
    // will grow over time at upgrade time with new fields (i.e. parametrized through chainspec)
    // those assertions will fail as legacy should use default values for new fields, and modern
    // protocol data should use new upgraded fields.
    assert_eq!(legacy_protocol_data, protocol_data_v1_3_0);
    assert_eq!(legacy_protocol_data, protocol_data_v1_4_0);
}
