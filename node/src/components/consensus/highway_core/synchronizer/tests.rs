use std::collections::BTreeSet;

use derive_more::Display;

use crate::components::consensus::{
    highway_core::{
        highway::{tests::test_validators, ValidVertex},
        highway_testing::TEST_INSTANCE_ID,
        state::{tests::*, State},
    },
    BlockContext,
};

use super::*;

#[derive(DataSize, Debug, Ord, PartialOrd, Copy, Clone, Display, Hash, Eq, PartialEq)]
pub(crate) struct NodeId(pub u8);

#[test]
fn purge_vertices() {
    let params = test_params(0);
    let mut state = State::new(WEIGHTS, params.clone(), vec![], vec![]);

    // We use round exponent 4u8, so a round is 0x10 ms. With seed 0, Carol is the first leader.
    //
    // time:  0x00 0x0A 0x1A 0x2A 0x3A
    //
    // Carol   c0 — c1 — c2
    //            \
    // Bob          ————————— b0 — b1
    let c0 = add_unit!(state, CAROL, 0x00, 4u8, 0xA; N, N, N).unwrap();
    let c1 = add_unit!(state, CAROL, 0x0A, 4u8, None; N, N, c0).unwrap();
    let c2 = add_unit!(state, CAROL, 0x1A, 4u8, None; N, N, c1).unwrap();
    let b0 = add_unit!(state, BOB, 0x2A, 4u8, None; N, N, c0).unwrap();
    let b1 = add_unit!(state, BOB, 0x3A, 4u8, None; N, b0, c0).unwrap();

    // A Highway instance that's just used to create PreValidatedVertex instances below.
    let util_highway =
        Highway::<TestContext>::new(TEST_INSTANCE_ID, test_validators(), params.clone());

    // Returns the WireUnit with the specified hash.
    let unit = |hash: u64| Vertex::Unit(state.wire_unit(&hash, TEST_INSTANCE_ID).unwrap());
    // Returns the PreValidatedVertex with the specified hash.
    let pvv = |hash: u64| util_highway.pre_validate_vertex(unit(hash)).unwrap();

    let peer0 = NodeId(0);

    // Create a synchronizer with a 0x20 ms timeout, and a Highway instance.
    let mut sync = Synchronizer::<NodeId, TestContext>::new(
        HighwayConfig {
            pending_vertex_timeout: 0x20.into(),
            ..Default::default()
        },
        WEIGHTS.len(),
        TEST_INSTANCE_ID,
    );
    let mut highway = Highway::<TestContext>::new(TEST_INSTANCE_ID, test_validators(), params);

    // At time 0x20, we receive c2, b0 and b1 — the latter ahead of their timestamp.
    // Since c2 is the first entry in the main queue, processing is scheduled.
    let now = 0x20.into();
    assert!(matches!(
        *sync.schedule_add_vertex(peer0, pvv(c2), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    sync.store_vertex_for_addition_later(unit(b1).timestamp().unwrap(), now, peer0, pvv(b1));
    sync.store_vertex_for_addition_later(unit(b0).timestamp().unwrap(), now, peer0, pvv(b0));

    // At time 0x21, we receive c1.
    let now = 0x21.into();
    assert!(sync.schedule_add_vertex(peer0, pvv(c1), now).is_empty());

    // No new vertices can be added yet, because all are missing dependencies.
    // The missing dependencies of c1 and c2 are requested.
    let (maybe_pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(maybe_pv.is_none());
    let requested_deps: BTreeSet<_> = outcomes
        .iter()
        .map(|outcome| match outcome {
            ProtocolOutcome::CreatedTargetedMessage(msg, peer) => {
                match bincode::deserialize(msg).unwrap() {
                    HighwayMessage::RequestDependency::<TestContext>(dep) => (dep, *peer),
                    msg => panic!("unexpected message: {:?}", msg),
                }
            }
            outcome => panic!("unexpected outcome: {:?}", outcome),
        })
        .collect();
    let expected_deps: BTreeSet<_> = vec![
        (Dependency::Unit(c0), peer0),
        (Dependency::UnitBySeqNum(0, CAROL), peer0),
    ]
    .into_iter()
    .collect();
    assert_eq!(expected_deps, requested_deps);

    // At 0x23, c0 gets enqueued and added.
    // That puts c1 back into the main queue, since its dependency is satisfied.
    let now = 0x23.into();
    let outcomes = sync.schedule_add_vertex(peer0, pvv(c0), now);
    assert!(
        matches!(*outcomes, [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]),
        "unexpected outcomes: {:?}",
        outcomes
    );
    let (maybe_pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert_eq!(Dependency::Unit(c0), maybe_pv.unwrap().vertex().id());
    assert!(outcomes.is_empty());
    let vv_c0 = highway.validate_vertex(pvv(c0)).expect("c0 is valid");
    highway.add_valid_vertex(vv_c0, now);
    let outcomes = sync.remove_satisfied_deps(&highway);
    assert!(
        matches!(*outcomes, [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]),
        "unexpected outcomes: {:?}",
        outcomes
    );

    // At time 0x2A, the vertex b0 moves into the main queue.
    let now = 0x2A.into();
    assert!(sync.add_past_due_stored_vertices(now).is_empty());

    // At 0x41, all vertices received at 0x20 are expired, but c1 (received at 0x21) isn't.
    // This will remove:
    // * b1: still postponed due to future timestamp
    // * b0: in the main queue
    // * c2: waiting for dependency c1 to be added
    sync.purge_vertices(0x41.into());

    // The main queue should now contain only c1. If we remove it, the synchronizer is empty.
    let (maybe_pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert_eq!(Dependency::Unit(c1), maybe_pv.unwrap().vertex().id());
    assert!(outcomes.is_empty());
    assert!(sync.is_empty());
}

#[test]
/// Test that when a vertex depends on a dependency that has already been synchronized, and is
/// waiting in the synchronizer queue state, but is not yet added to the protocol state – that we
/// don't request it again.
fn do_not_download_synchronized_dependencies() {
    let params = test_params(0);
    // A Highway and state instances that are used to create PreValidatedVertex instances below.

    let mut state = State::new(WEIGHTS, params.clone(), vec![], vec![]);
    let util_highway =
        Highway::<TestContext>::new(TEST_INSTANCE_ID, test_validators(), params.clone());

    // We use round exponent 4u8, so a round is 0x10 ms. With seed 0, Carol is the first leader.
    //
    // time:  0x00 0x0A 0x1A 0x2A 0x3A
    //
    // Carol   c0 — c1 — c2
    //                \
    // Bob             — b0

    let c0 = add_unit!(state, CAROL, 0x00, 4u8, 0xA; N, N, N).unwrap();
    let c1 = add_unit!(state, CAROL, 0x0A, 4u8, None; N, N, c0).unwrap();
    let c2 = add_unit!(state, CAROL, 0x1A, 4u8, None; N, N, c1).unwrap();
    let b0 = add_unit!(state, BOB, 0x2A, 4u8, None; N, N, c1).unwrap();

    // Returns the WireUnit with the specified hash.
    let unit = |hash: u64| Vertex::Unit(state.wire_unit(&hash, TEST_INSTANCE_ID).unwrap());
    let unit_with_panorama = |hash: u64| {
        Vertex::UnitWithPanorama(
            state.wire_unit(&hash, TEST_INSTANCE_ID).unwrap(),
            state.unit(&hash).panorama.clone(),
        )
    };
    // Returns the PreValidatedVertex with the specified hash.
    let pvv = |hash: u64| util_highway.pre_validate_vertex(unit(hash)).unwrap();
    let pvv_with_panorama = |hash: u64| {
        util_highway
            .pre_validate_vertex(unit_with_panorama(hash))
            .unwrap()
    };

    let peer0 = NodeId(0);
    let peer1 = NodeId(1);

    // Create a synchronizer with a 0x20 ms timeout, and a Highway instance.
    let mut sync = Synchronizer::<NodeId, TestContext>::new(
        HighwayConfig {
            pending_vertex_timeout: 0x20.into(),
            ..Default::default()
        },
        WEIGHTS.len(),
        TEST_INSTANCE_ID,
    );

    let mut highway = Highway::<TestContext>::new(TEST_INSTANCE_ID, test_validators(), params);
    let now = 0x20.into();

    // We add the units with full panorama: Deduplication only works if the synchronizer can be
    // sure that the downloaded unit matches, i.e. if it is known by hash.
    assert!(matches!(
        *sync.schedule_add_vertex(peer0, pvv_with_panorama(c2), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    // `c2` can't be added to the protocol state yet b/c it's missing its `c1` dependency.
    let (pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(pv.is_none());
    assert_targeted_message(
        &unwrap_single(outcomes),
        &peer0,
        HighwayMessage::RequestDependency(Dependency::Unit(c1)),
    );
    // Simulate `c1` being downloaded…
    let c1_outcomes = sync.schedule_add_vertex(peer0, pvv(c1), now);
    assert!(matches!(
        *c1_outcomes,
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    // `b0` can't be added to the protocol state b/c it's missing its `c1` dependency,
    // but `c1` has already been downloaded so we should not request it again. We will only request
    // `c0` as that's what `c1` depends on.
    let (pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(pv.is_none());
    assert_targeted_message(
        &unwrap_single(outcomes),
        &peer0,
        HighwayMessage::RequestDependency(Dependency::Unit(c0)),
    );
    // `c1` is now part of the synchronizer's state, we should not try requesting it if other
    // vertices depend on it.
    assert!(matches!(
        *sync.schedule_add_vertex(peer1, pvv_with_panorama(b0), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    let (pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(pv.is_none());
    // `b0` depends on `c1`, that is already in the synchronizer's state, but it also depends on
    // `c0` transitively that is not yet known. We should request it, even if we had already
    // done that for `c1`.
    assert_targeted_message(
        &unwrap_single(outcomes),
        &peer1,
        HighwayMessage::RequestDependency(Dependency::Unit(c0)),
    );
    // "Download" the last dependency.
    let _ = sync.schedule_add_vertex(peer0, pvv(c0), now);
    // Now, the whole chain can be added to the protocol state.
    let mut units: BTreeSet<Dependency<TestContext>> = vec![
        Dependency::Unit(c0),
        Dependency::Unit(c1),
        Dependency::UnitWithPanorama(b0),
        Dependency::UnitWithPanorama(c2),
    ]
    .into_iter()
    .collect();
    while let (Some(pv), outcomes) = sync.pop_vertex_to_add(&highway, &Default::default()) {
        // Verify that we don't request any dependency now.
        assert!(
            !outcomes
                .iter()
                .any(|outcome| matches!(outcome, ProtocolOutcome::CreatedTargetedMessage(_, _))),
            "unexpected dependency request {:?}",
            outcomes
        );
        let pv_dep = pv.vertex().id();
        assert!(units.remove(&pv_dep), "unexpected dependency: {:?}", pv_dep);
        match pv_dep {
            Dependency::Unit(hash) | Dependency::UnitWithPanorama(hash) => {
                let vv = highway
                    .validate_vertex(pvv(hash))
                    .unwrap_or_else(|_| panic!("{:?} unit is valid", hash));
                highway.add_valid_vertex(vv, now);
                let _ = sync.remove_satisfied_deps(&highway);
            }
            pv_dep => panic!("expected unit, got {:?}", pv_dep),
        }
    }
    assert!(sync.is_empty());
}

#[test]
fn transitive_proposal_dependency() {
    let params = test_params(0);
    // A Highway and state instances that are used to create PreValidatedVertex instances below.

    let mut state = State::new(WEIGHTS, params.clone(), vec![], vec![]);
    let util_highway =
        Highway::<TestContext>::new(TEST_INSTANCE_ID, test_validators(), params.clone());

    // We use round exponent 4u8, so a round is 0x10 ms. With seed 0, Carol is the first leader.
    //
    // time:  0x00 0x0A 0x1A 0x2A
    //
    //              a0'
    // Alice      /
    //           /  a0 ——
    //          / /       \
    // Carol   c0 — c1     \
    //                \     \
    // Bob             — b0 — b1

    let c0 = add_unit!(state, CAROL, 0x00, 4u8, 0xA; N, N, N).unwrap();
    let c1 = add_unit!(state, CAROL, 0x0A, 4u8, None; N, N, c0).unwrap();
    let a0 = add_unit!(state, ALICE, 0x0A, 4u8, None; N, N, c0).unwrap();
    let a0_prime = add_unit!(state, ALICE, 0x0A, 8u8, None; N, N, c0).unwrap();
    let b0 = add_unit!(state, BOB, 0x1A, 4u8, None; N, N, c1).unwrap();
    let b1 = add_unit!(state, BOB, 0x2A, 4u8, None; a0, b0, c1).unwrap();

    // Returns the WireUnit with the specified hash.
    let unit = |hash: u64| Vertex::Unit(state.wire_unit(&hash, TEST_INSTANCE_ID).unwrap());
    let unit_with_panorama = |hash: u64| {
        Vertex::UnitWithPanorama(
            state.wire_unit(&hash, TEST_INSTANCE_ID).unwrap(),
            state.unit(&hash).panorama.clone(),
        )
    };
    // Returns the PreValidatedVertex with the specified hash.
    let pvv = |hash: u64| util_highway.pre_validate_vertex(unit(hash)).unwrap();
    let pvv_with_panorama = |hash: u64| {
        util_highway
            .pre_validate_vertex(unit_with_panorama(hash))
            .unwrap()
    };
    let peer0 = NodeId(0);
    let peer1 = NodeId(1);

    // Create a synchronizer with a 0x20 ms timeout, and a Highway instance.
    let mut sync = Synchronizer::<NodeId, TestContext>::new(
        HighwayConfig {
            pending_vertex_timeout: 0x20.into(),
            ..Default::default()
        },
        WEIGHTS.len(),
        TEST_INSTANCE_ID,
    );

    let mut highway = Highway::<TestContext>::new(TEST_INSTANCE_ID, test_validators(), params);
    let now = 0x20.into();

    assert!(matches!(
        *sync.schedule_add_vertex(peer0, pvv(c1), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    // `c1` can't be added to the protocol state yet b/c it's missing its `c0` dependency.
    let (pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(pv.is_none());
    assert_targeted_message(
        &unwrap_single(outcomes),
        &peer0,
        HighwayMessage::RequestDependency(Dependency::Unit(c0)),
    );
    // "Download" and schedule addition of c0.
    let c0_outcomes = sync.schedule_add_vertex(peer0, pvv(c0), now);
    assert!(matches!(
        *c0_outcomes,
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    // `c0` has no dependencies so we can try adding it to the protocol state.
    let (maybe_pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    let pv = maybe_pv.expect("expected c0 vertex");
    assert_eq!(pv.vertex(), &unit(c0));
    assert!(outcomes.is_empty());
    // `b0` can't be added either b/c it's relying on `c1` and `c0`.
    // We add `b0` with its full panorama: Then the synchronizer can know that it depends on `c1`
    // and should detect the transitive dependency.
    assert!(matches!(
        *sync.schedule_add_vertex(peer1, pvv_with_panorama(b0), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    let c0_pending_values = {
        let mut tmp = HashMap::new();
        let vv = ValidVertex(unit(c0));
        let proposed_block = ProposedBlock::new(1u32, BlockContext::new(now, Vec::new()));
        let mut set = HashSet::new();
        set.insert((vv, peer0));
        tmp.insert(proposed_block, set);
        tmp
    };
    let (maybe_pv, outcomes) = sync.pop_vertex_to_add(&highway, &c0_pending_values);
    let pv = maybe_pv.unwrap();
    assert_eq!(pv.sender(), &peer1);
    assert_eq!(pv.vertex(), &unit(c0));
    // `b0` depends on `c1` and `c0` transitively but `c0`'s deploys are being downloaded,
    // so we don't re-request it.
    assert!(outcomes.is_empty());

    let vv_c0 = highway.validate_vertex(pvv(c0)).expect("c0 is valid");
    highway.add_valid_vertex(vv_c0, now);
    let vv_c1 = highway.validate_vertex(pvv(c1)).expect("c1 is valid");
    highway.add_valid_vertex(vv_c1, now);
    let vv_b0 = highway.validate_vertex(pvv(b0)).expect("b0 is valid");
    highway.add_valid_vertex(vv_b0, now);
    let vv_a0p = highway
        .validate_vertex(pvv(a0_prime))
        .expect("a0' is valid");
    highway.add_valid_vertex(vv_a0p, now);

    // We added the wrong fork of Alice. We'll fail to compute the panorama of b1 and request it.
    assert!(matches!(
        *sync.schedule_add_vertex(peer0, pvv(b1), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    let (pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(pv.is_none());
    assert_targeted_message(
        &unwrap_single(outcomes),
        &peer0,
        HighwayMessage::RequestDependency(Dependency::UnitWithPanorama(b1)),
    );

    // Once we received the full panorama, we request a0 by hash.
    assert!(matches!(
        *sync.schedule_add_vertex(peer0, pvv_with_panorama(b1), now),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    assert!(sync.remove_satisfied_deps(&highway).is_empty());
    let (pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    assert!(pv.is_none());
    assert_targeted_message(
        &unwrap_single(outcomes),
        &peer0,
        HighwayMessage::RequestDependency(Dependency::Unit(a0)),
    );

    // With a0 added, we can finally add b1 as well.
    let vv_a0 = highway.validate_vertex(pvv(a0)).expect("a0 is valid");
    highway.add_valid_vertex(vv_a0, now);
    assert!(matches!(
        *sync.remove_satisfied_deps(&highway),
        [ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
    ));
    let (maybe_pv, outcomes) = sync.pop_vertex_to_add(&highway, &Default::default());
    let pv = maybe_pv.unwrap();
    assert_eq!(pv.sender(), &peer0);
    assert_eq!(pv.vertex(), &unit_with_panorama(b1));
    assert!(outcomes.is_empty());
}

fn unwrap_single<T: Debug>(vec: Vec<T>) -> T {
    assert_eq!(
        vec.len(),
        1,
        "expected single element in the vector {:?}",
        vec
    );
    vec.into_iter().next().unwrap()
}

fn assert_targeted_message(
    outcome: &ProtocolOutcome<NodeId, TestContext>,
    peer: &NodeId,
    expected: HighwayMessage<TestContext>,
) {
    match outcome {
        ProtocolOutcome::CreatedTargetedMessage(msg, peer0) => {
            assert_eq!(peer, peer0);
            let highway_message: HighwayMessage<TestContext> =
                bincode::deserialize(msg.as_slice()).expect("deserialization to pass");
            assert_eq!(highway_message, expected);
        }
        _ => panic!("unexpected outcome: {:?}", outcome),
    }
}
