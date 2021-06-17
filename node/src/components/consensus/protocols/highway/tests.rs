use std::{collections::BTreeSet, sync::Arc};

use datasize::DataSize;
use derive_more::Display;

use casper_types::{PublicKey, U512};

use crate::{
    components::consensus::{
        cl_context::{ClContext, Keypair},
        config::Config,
        consensus_protocol::{ConsensusProtocol, ProtocolOutcome},
        highway_core::{
            highway::{SignedWireUnit, Vertex, WireUnit},
            highway_testing,
            state::{self, tests::ALICE, Observation, Panorama},
            validators::ValidatorIndex,
            State,
        },
        protocols::highway::{
            config::Config as HighwayConfig, HighwayMessage, ACTION_ID_VERTEX,
            TIMER_ID_STANDSTILL_ALERT,
        },
        tests::utils::{new_test_chainspec, ALICE_PUBLIC_KEY, ALICE_SECRET_KEY, BOB_PUBLIC_KEY},
        traits::Context,
        HighwayProtocol,
    },
    types::{BlockPayload, TimeDiff, Timestamp},
};

#[derive(DataSize, Debug, Ord, PartialOrd, Copy, Clone, Display, Hash, Eq, PartialEq)]
pub(crate) struct NodeId(pub u8);

/// Returns a new `State` with `ClContext` parameters suitable for tests.
pub(crate) fn new_test_state<I, T>(weights: I, seed: u64) -> State<ClContext>
where
    I: IntoIterator<Item = T>,
    T: Into<state::Weight>,
{
    let params = state::Params::new(
        seed,
        highway_testing::TEST_BLOCK_REWARD,
        highway_testing::TEST_BLOCK_REWARD / 5,
        14,
        19,
        4,
        u64::MAX,
        0.into(),
        Timestamp::from(u64::MAX),
        highway_testing::TEST_ENDORSEMENT_EVIDENCE_LIMIT,
    );
    let weights = weights.into_iter().map(|w| w.into()).collect::<Vec<_>>();
    state::State::new(weights, params, vec![], vec![])
}

const INSTANCE_ID_DATA: &[u8; 1] = &[123u8; 1];
const STANDSTILL_TIMEOUT: &str = "1min";

pub(crate) fn new_test_highway_protocol<I1, I2, T>(
    weights: I1,
    init_slashed: I2,
) -> Box<dyn ConsensusProtocol<NodeId, ClContext>>
where
    I1: IntoIterator<Item = (PublicKey, T)>,
    I2: IntoIterator<Item = PublicKey>,
    T: Into<U512>,
{
    let weights = weights
        .into_iter()
        .map(|(pk, w)| (pk, w.into()))
        .collect::<Vec<_>>();
    let chainspec = new_test_chainspec(weights.clone());
    let config = Config {
        secret_key_path: Default::default(),
        highway: HighwayConfig {
            pending_vertex_timeout: "1min".parse().unwrap(),
            standstill_timeout: STANDSTILL_TIMEOUT.parse().unwrap(),
            log_participation_interval: "10sec".parse().unwrap(),
            max_execution_delay: 3,
            ..HighwayConfig::default()
        },
    };
    // Timestamp of the genesis era start and test start.
    let start_timestamp: Timestamp = 0.into();
    let (hw_proto, outcomes) = HighwayProtocol::<NodeId, ClContext>::new_boxed(
        ClContext::hash(INSTANCE_ID_DATA),
        weights.into_iter().collect(),
        &init_slashed.into_iter().collect(),
        &None.into_iter().collect(),
        &(&chainspec).into(),
        &config,
        None,
        start_timestamp,
        0,
        start_timestamp,
    );
    // We expect for messages:
    // * log participation timer,
    // * log synchronizer queue length timer,
    // * purge synchronizer queue timer,
    // * inactivity timer,
    // If there are more, the tests might need to handle them.
    assert_eq!(4, outcomes.len());
    hw_proto
}

#[test]
fn test_highway_protocol_handle_message_parse_error() {
    // Build a highway_protocol for instrumentation
    let mut highway_protocol: Box<dyn ConsensusProtocol<NodeId, ClContext>> =
        new_test_highway_protocol(vec![(ALICE_PUBLIC_KEY.clone(), 100)], vec![]);

    let now = Timestamp::zero();
    let sender = NodeId(123);
    let msg = vec![];
    let mut effects: Vec<ProtocolOutcome<NodeId, ClContext>> =
        highway_protocol.handle_message(sender.to_owned(), msg.to_owned(), now);

    assert_eq!(effects.len(), 1);

    let maybe_protocol_outcome = effects.pop();

    match &maybe_protocol_outcome {
        None => panic!("We just checked that effects has length 1!"),
        Some(ProtocolOutcome::InvalidIncomingMessage(invalid_msg, offending_sender, _err)) => {
            assert_eq!(
                invalid_msg, &msg,
                "Invalid message is not message that was sent."
            );
            assert_eq!(offending_sender, &sender, "Unexpected sender.")
        }
        Some(protocol_outcome) => panic!("Unexpected protocol outcome {:?}", protocol_outcome),
    }
}

pub(crate) const N: Observation<ClContext> = Observation::None;

#[test]
fn send_a_wire_unit_with_too_small_a_round_exp() {
    let creator: ValidatorIndex = ValidatorIndex(0);
    let validators = vec![(ALICE_PUBLIC_KEY.clone(), 100)];
    let state: State<ClContext> = new_test_state(validators.iter().map(|(_pk, w)| *w), 0);
    let panorama: Panorama<ClContext> = Panorama::from(vec![N]);
    let seq_number = panorama.next_seq_num(&state, creator);
    let now = Timestamp::zero();
    let panorama_hash = panorama.hash();
    let seq_num_panorama = panorama.to_seq_num_panorama(&state);
    let wunit: WireUnit<ClContext> = WireUnit {
        seq_num_panorama,
        panorama_hash,
        previous: None,
        creator,
        instance_id: ClContext::hash(INSTANCE_ID_DATA),
        value: None,
        seq_number,
        timestamp: now,
        round_exp: 0,
        endorsed: BTreeSet::new(),
    };
    let alice_keypair: Keypair = Keypair::from(Arc::clone(&*ALICE_SECRET_KEY));
    let highway_message: HighwayMessage<ClContext> = HighwayMessage::NewVertex(Vertex::Unit(
        SignedWireUnit::new(wunit.into_hashed(), &alice_keypair),
    ));
    let mut highway_protocol = new_test_highway_protocol(validators, vec![]);
    let sender = NodeId(123);
    let msg = bincode::serialize(&highway_message).unwrap();
    let mut outcomes = highway_protocol.handle_message(sender.to_owned(), msg.to_owned(), now);
    assert_eq!(outcomes.len(), 1);

    let maybe_protocol_outcome = outcomes.pop();
    match &maybe_protocol_outcome {
        None => unreachable!("We just checked that outcomes has length 1!"),
        Some(ProtocolOutcome::InvalidIncomingMessage(invalid_msg, offending_sender, err)) => {
            assert_eq!(
                invalid_msg, &msg,
                "Invalid message is not message that was sent."
            );
            assert_eq!(offending_sender, &sender, "Unexpected sender.");
            assert!(
                format!("{:?}", err).starts_with(
                    "The vertex contains an invalid unit: `The round \
                     length exponent is less than the minimum allowed by \
                     the chain-spec.`"
                ),
                "Error message did not start as expected: {:?}",
                err
            )
        }
        Some(protocol_outcome) => panic!("Unexpected protocol outcome {:?}", protocol_outcome),
    }
}

#[test]
fn send_a_valid_wire_unit() {
    let standstill_timeout: TimeDiff = STANDSTILL_TIMEOUT.parse().unwrap();
    let creator: ValidatorIndex = ValidatorIndex(0);
    let validators = vec![(ALICE_PUBLIC_KEY.clone(), 100)];
    let state: State<ClContext> = new_test_state(validators.iter().map(|(_pk, w)| *w), 0);
    let panorama: Panorama<ClContext> = Panorama::from(vec![N]);
    let seq_number = panorama.next_seq_num(&state, creator);
    let panorama_hash = panorama.hash();
    let seq_num_panorama = panorama.to_seq_num_panorama(&state);
    let mut now = Timestamp::zero();
    let wunit: WireUnit<ClContext> = WireUnit {
        seq_num_panorama,
        panorama_hash,
        previous: None,
        creator,
        instance_id: ClContext::hash(INSTANCE_ID_DATA),
        value: Some(Arc::new(BlockPayload::new(vec![], vec![], vec![], false))),
        seq_number,
        timestamp: now,
        round_exp: 14,
        endorsed: BTreeSet::new(),
    };
    let alice_keypair: Keypair = Keypair::from(Arc::clone(&*ALICE_SECRET_KEY));
    let highway_message: HighwayMessage<ClContext> = HighwayMessage::NewVertex(Vertex::Unit(
        SignedWireUnit::new(wunit.into_hashed(), &alice_keypair),
    ));

    let mut highway_protocol = new_test_highway_protocol(validators, vec![]);
    let sender = NodeId(123);
    let msg = bincode::serialize(&highway_message).unwrap();

    let mut outcomes = highway_protocol.handle_message(sender, msg, now);
    while let Some(outcome) = outcomes.pop() {
        match outcome {
            ProtocolOutcome::CreatedGossipMessage(_) | ProtocolOutcome::FinalizedBlock(_) => (),
            ProtocolOutcome::QueueAction(ACTION_ID_VERTEX) => {
                outcomes.extend(highway_protocol.handle_action(ACTION_ID_VERTEX, now))
            }
            outcome => panic!("Unexpected outcome: {:?}", outcome),
        }
    }

    // Our protocol state has changed since initialization, so there is no alert.
    now += standstill_timeout;
    let outcomes = highway_protocol.handle_timer(now, TIMER_ID_STANDSTILL_ALERT);
    match &*outcomes {
        [ProtocolOutcome::ScheduleTimer(timestamp, timer_id)] => {
            assert_eq!(*timestamp, now + standstill_timeout);
            assert_eq!(*timer_id, TIMER_ID_STANDSTILL_ALERT);
        }
        _ => panic!("Unexpected outcomes: {:?}", outcomes),
    }

    // If after another timeout, the state has not changed, an alert is raised.
    now += standstill_timeout;
    let outcomes = highway_protocol.handle_timer(now, TIMER_ID_STANDSTILL_ALERT);
    assert!(
        matches!(&*outcomes, [ProtocolOutcome::StandstillAlert]),
        "Unexpected outcomes: {:?}",
        outcomes
    );
}

#[test]
fn detect_doppelganger() {
    let creator: ValidatorIndex = ALICE;
    let validators = vec![
        (ALICE_PUBLIC_KEY.clone(), 100),
        (BOB_PUBLIC_KEY.clone(), 100),
    ];
    let state: State<ClContext> = new_test_state(validators.iter().map(|(_pk, w)| *w), 0);
    let panorama: Panorama<ClContext> = Panorama::from(vec![N, N]);
    let seq_number = panorama.next_seq_num(&state, creator);
    let instance_id = ClContext::hash(INSTANCE_ID_DATA);
    let round_exp = 14;
    let now = Timestamp::zero();
    let value = Arc::new(BlockPayload::new(vec![], vec![], vec![], false));
    let panorama_hash = panorama.hash();
    let seq_num_panorama = panorama.to_seq_num_panorama(&state);
    let wunit: WireUnit<ClContext> = WireUnit {
        seq_num_panorama,
        panorama_hash,
        previous: None,
        creator,
        instance_id,
        value: Some(value),
        seq_number,
        timestamp: now,
        round_exp,
        endorsed: BTreeSet::new(),
    };
    let alice_keypair: Keypair = Keypair::from(Arc::clone(&*ALICE_SECRET_KEY));
    let highway_message: HighwayMessage<ClContext> = HighwayMessage::NewVertex(Vertex::Unit(
        SignedWireUnit::new(wunit.into_hashed(), &alice_keypair),
    ));
    let mut highway_protocol = new_test_highway_protocol(validators, vec![]);
    // Activate ALICE as validator.
    let _ = highway_protocol.activate_validator(ALICE_PUBLIC_KEY.clone(), alice_keypair, now, None);
    assert_eq!(highway_protocol.is_active(), true);
    let sender = NodeId(123);
    let msg = bincode::serialize(&highway_message).unwrap();
    // "Send" a message created by ALICE to an instance of Highway where she's an active validator.
    // An incoming unit, created by the same validator, should be properly detected as a
    // doppelganger.
    let mut outcomes = highway_protocol.handle_message(sender, msg, now);
    while let Some(outcome) = outcomes.pop() {
        match outcome {
            ProtocolOutcome::DoppelgangerDetected => return,
            ProtocolOutcome::QueueAction(ACTION_ID_VERTEX) => {
                outcomes.extend(highway_protocol.handle_action(ACTION_ID_VERTEX, now))
            }
            _ => (),
        }
    }
    panic!("failed to return DoppelgangerDetected effect");
}
