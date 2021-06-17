#![allow(unused_qualifications)] // This is to suppress warnings originating in the test macros.
#![allow(clippy::integer_arithmetic)] // Overflows in tests would panic anyway.

use std::{
    collections::{hash_map::DefaultHasher, BTreeSet},
    hash::Hasher,
};

use datasize::DataSize;
use rand::{Rng, RngCore};

use super::*;
use crate::components::consensus::{
    highway_core::{
        evidence::EvidenceError,
        highway::Dependency,
        highway_testing::{TEST_BLOCK_REWARD, TEST_ENDORSEMENT_EVIDENCE_LIMIT, TEST_INSTANCE_ID},
    },
    traits::{ConsensusValueT, ValidatorSecret},
};

pub(crate) const WEIGHTS: &[Weight] = &[Weight(3), Weight(4), Weight(5)];

pub(crate) const ALICE: ValidatorIndex = ValidatorIndex(0);
pub(crate) const BOB: ValidatorIndex = ValidatorIndex(1);
pub(crate) const CAROL: ValidatorIndex = ValidatorIndex(2);
pub(crate) const DAN: ValidatorIndex = ValidatorIndex(3);
pub(crate) const ERIC: ValidatorIndex = ValidatorIndex(4);
pub(crate) const FRANK: ValidatorIndex = ValidatorIndex(5);
pub(crate) const GINA: ValidatorIndex = ValidatorIndex(6);
pub(crate) const HANNA: ValidatorIndex = ValidatorIndex(7);

pub(crate) const N: Observation<TestContext> = Observation::None;
pub(crate) const F: Observation<TestContext> = Observation::Faulty;

const TEST_MIN_ROUND_EXP: u8 = 4;
const TEST_MAX_ROUND_EXP: u8 = 19;
const TEST_INIT_ROUND_EXP: u8 = 4;
const TEST_ERA_HEIGHT: u64 = 5;

#[derive(Clone, DataSize, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TestContext;

#[derive(Clone, DataSize, Debug, Eq, PartialEq)]
pub(crate) struct TestSecret(pub(crate) u32);

impl ValidatorSecret for TestSecret {
    type Hash = u64;
    type Signature = u64;

    fn sign(&self, data: &Self::Hash) -> Self::Signature {
        data + u64::from(self.0)
    }
}

pub(crate) const ALICE_SEC: TestSecret = TestSecret(0);
pub(crate) const BOB_SEC: TestSecret = TestSecret(1);
pub(crate) const CAROL_SEC: TestSecret = TestSecret(2);
pub(crate) const DAN_SEC: TestSecret = TestSecret(3);

impl ConsensusValueT for u32 {
    fn needs_validation(&self) -> bool {
        false
    }
}

impl Context for TestContext {
    type ConsensusValue = u32;
    type ValidatorId = u32;
    type ValidatorSecret = TestSecret;
    type Signature = u64;
    type Hash = u64;
    type InstanceId = u64;

    fn hash(data: &[u8]) -> Self::Hash {
        let mut hasher = DefaultHasher::new();
        hasher.write(data);
        hasher.finish()
    }

    fn verify_signature(
        hash: &Self::Hash,
        public_key: &Self::ValidatorId,
        signature: &<Self::ValidatorSecret as ValidatorSecret>::Signature,
    ) -> bool {
        let computed_signature = hash + u64::from(*public_key);
        computed_signature == *signature
    }
}

impl From<<TestContext as Context>::Hash> for Observation<TestContext> {
    fn from(vhash: <TestContext as Context>::Hash) -> Self {
        Observation::Correct(vhash)
    }
}

/// Returns the cause of the error, dropping the `WireUnit`.
fn unit_err(err: AddUnitError<TestContext>) -> UnitError {
    err.cause
}

/// An error that occurred when trying to add a unit.
#[derive(Debug, Error)]
#[error("{:?}", .cause)]
pub(crate) struct AddUnitError<C: Context> {
    /// The invalid unit that was not added to the protocol state.
    pub(crate) swunit: SignedWireUnit<C>,
    /// The reason the unit is invalid.
    #[source]
    pub(crate) cause: UnitError,
}

impl<C: Context> SignedWireUnit<C> {
    fn with_error(self, cause: UnitError) -> AddUnitError<C> {
        AddUnitError {
            swunit: self,
            cause,
        }
    }
}

pub(crate) fn test_params(seed: u64) -> Params {
    Params::new(
        seed,
        TEST_BLOCK_REWARD,
        TEST_BLOCK_REWARD / 5,
        TEST_MIN_ROUND_EXP,
        TEST_MAX_ROUND_EXP,
        TEST_INIT_ROUND_EXP,
        TEST_ERA_HEIGHT,
        Timestamp::from(0),
        Timestamp::from(0),
        TEST_ENDORSEMENT_EVIDENCE_LIMIT,
    )
}

impl State<TestContext> {
    /// Returns a new `State` with `TestContext` parameters suitable for tests.
    pub(crate) fn new_test(weights: &[Weight], seed: u64) -> Self {
        State::new(weights, test_params(seed), vec![], vec![])
    }

    /// Adds the unit to the protocol state, or returns an error if it is invalid.
    /// Panics if dependencies are not satisfied.
    pub(crate) fn add_unit(
        &mut self,
        swunit: SignedWireUnit<TestContext>,
        panorama: Panorama<TestContext>,
    ) -> Result<(), AddUnitError<TestContext>> {
        if let Err(err) = self.pre_validate_unit(&swunit, Some(&panorama)) {
            return Err(swunit.with_error(err));
        }
        assert_eq!(None, panorama.missing_dependency(self));
        if let Err(err) = self.validate_unit(&swunit, &panorama) {
            return Err(swunit.with_error(err));
        }
        assert_eq!(None, self.needs_endorsements(&swunit));
        self.add_valid_unit(swunit, panorama);
        Ok(())
    }
}

#[test]
fn add_unit() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);

    // Create units as follows; a0, b0 are blocks:
    //
    // Alice: a0 ————— a1
    //                /
    // Bob:   b0 —— b1
    //          \  /
    // Carol:    c0
    let a0 = add_unit!(state, ALICE, 0xA; N, N, N)?;
    let b0 = add_unit!(state, BOB, 48, 4u8, 0xB; N, N, N)?;
    let c0 = add_unit!(state, CAROL, 49, 4u8, None; N, b0, N)?;
    let b1 = add_unit!(state, BOB, 49, 4u8, None; N, b0, c0)?;
    let _a1 = add_unit!(state, ALICE, None; a0, b1, c0)?;

    // Wrong sequence number: Bob hasn't produced b2 yet.
    let panorama = panorama!(N, b1, c0);
    let panorama_hash = panorama.hash();
    let seq_num_panorama = panorama.to_seq_num_panorama(&state);
    let mut wunit = WireUnit {
        seq_num_panorama,
        panorama_hash,
        previous: Some(b1),
        creator: BOB,
        instance_id: TEST_INSTANCE_ID,
        value: None,
        seq_number: 3,
        timestamp: 51.into(),
        round_exp: 4u8,
        endorsed: BTreeSet::new(),
    };
    let unit = SignedWireUnit::new(wunit.clone().into_hashed(), &BOB_SEC);
    let maybe_err = state.add_unit(unit, panorama.clone()).err().map(unit_err);
    assert_eq!(Some(UnitError::SequenceNumber), maybe_err);

    // Still not valid: This would be the third unit in the first round.
    wunit.seq_number = 2;
    let unit = SignedWireUnit::new(wunit.clone().into_hashed(), &BOB_SEC);
    let maybe_err = state.add_unit(unit, panorama).err().map(unit_err);
    assert_eq!(Some(UnitError::ThreeUnitsInRound), maybe_err);
    wunit.timestamp = 65.into();

    // Your sequence number must match the panorama.
    wunit.seq_number = 3;
    let unit = SignedWireUnit::new(wunit.clone().into_hashed(), &BOB_SEC);
    let maybe_err = state.pre_validate_unit(&unit, None).err();
    assert_eq!(Some(UnitError::SequenceNumber), maybe_err);
    wunit.seq_number = 2;

    // If a hash-based panorama is included it must match the sequence number-based one.
    let unit = SignedWireUnit::new(wunit.into_hashed(), &BOB_SEC);
    let maybe_err = state
        .add_unit(unit.clone(), panorama!(a0, b1, c0))
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::PanoramaMismatch(ALICE)), maybe_err);
    let maybe_err = state
        .add_unit(unit.clone(), panorama!(N, b1, c0, N))
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::PanoramaLength(4)), maybe_err);

    // Your previous unit must match the panorama.
    let maybe_err = state
        .add_unit(unit, panorama!(a0, N, c0))
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::PreviousUnit), maybe_err);

    // Inconsistent panorama: If you see b1, you have to see c0, too.
    let maybe_err = add_unit!(state, CAROL, None; N, b1, N).err().map(unit_err);
    assert_eq!(Some(UnitError::InconsistentPanorama(BOB)), maybe_err);

    // You can't make the round exponent too small.
    let maybe_err = add_unit!(state, CAROL, 50, 5u8, None; N, b1, c0)
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::RoundLengthExpChangedWithinRound), maybe_err);

    // And you can't make the round exponent too big.
    let maybe_err = add_unit!(state, CAROL, 50, 40u8, None; N, b1, c0)
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::RoundLengthExpGreaterThanMaximum), maybe_err);

    // After the round from 48 to 64 has ended, the exponent can change.
    let c1 = add_unit!(state, CAROL, 65, 5u8, None; N, b1, c0)?;

    // If you don't cite any other units, you must propose a block.
    let maybe_err = add_unit!(state, CAROL, 0, 4u8, None; N, N, N)
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::MissingBlock), maybe_err);

    // You can't cite yourself as faulty.
    let maybe_err = add_unit!(state, CAROL, 0, 4u8, 0xC; N, N, F)
        .err()
        .map(unit_err);
    assert_eq!(Some(UnitError::FaultyCreator), maybe_err);

    // Alice has not equivocated yet, and not produced message A1.
    let missing = panorama!(F, b1, c0).missing_dependency(&state);
    assert_eq!(Some(Dependency::Evidence(ALICE)), missing);
    let missing = panorama!(42, b1, c0).missing_dependency(&state);
    assert_eq!(Some(Dependency::Unit(42)), missing);

    // Alice equivocates: ae1 doesn't see a1.
    let ae1 = add_unit!(state, ALICE, 0xAE1; a0, b1, c0)?;
    assert!(state.has_evidence(ALICE));
    assert_eq!(panorama![F, b1, c1], *state.panorama());

    let missing = panorama!(F, b1, c0).missing_dependency(&state);
    assert_eq!(None, missing);
    let missing = panorama!(ae1, b1, c0).missing_dependency(&state);
    assert_eq!(None, missing);

    // Bob can see the equivocation.
    let b2 = add_unit!(state, BOB, None; F, b1, c0)?;

    // The state's own panorama has been updated correctly.
    assert_eq!(*state.panorama(), panorama!(F, b2, c1));
    Ok(())
}

#[test]
fn compute_panorama() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);

    // Create units as follows; a0, b0 are blocks:
    //
    // Alice: a0 — a1 — a2
    //                 /
    // Bob:   b0 ——— b1
    //          \   /
    //            c0
    // Carol:
    //            c0'
    let a0 = add_unit!(state, ALICE, 0xA; N, N, N)?;
    let a1 = add_unit!(state, ALICE, None; a0, N, N)?;
    let b0 = add_unit!(state, BOB, 48, 4u8, 0xB; N, N, N)?;
    let c0 = add_unit!(state, CAROL, 49, 4u8, None; N, b0, N)?;
    let c0_prime = add_unit!(state, CAROL, 50, 4u8, None; N, b0, N)?;
    let b1 = add_unit!(state, BOB, 49, 4u8, None; N, b0, c0)?;
    let a2 = add_unit!(state, ALICE, None; a1, b1, F)?;

    // Now we try to add a2 to a new state.
    let mut new_state = State::new_test(WEIGHTS, 0);

    // First it should request all of the creator's — Alice's — own units.
    let a2_swunit = state.wire_unit(&a2, TEST_INSTANCE_ID).unwrap();
    let res = new_state.compute_panorama(&a2_swunit);
    assert_eq!(Err(Dependency::UnitBySeqNum(0, ALICE)), res);
    let a0_swunit = state.wire_unit(&a0, TEST_INSTANCE_ID).unwrap();
    let a0_panorama = new_state.compute_panorama(&a0_swunit).unwrap();
    new_state.add_unit(a0_swunit, a0_panorama)?;

    // Once a0 is added, it requests a1. Since a2 knows its predecessor a1 by hash, it doesn't
    // request it by sequence number.
    let res = new_state.compute_panorama(&a2_swunit);
    assert_eq!(Err(Dependency::Unit(a1)), res);
    let a1_swunit = state.wire_unit(&a1, TEST_INSTANCE_ID).unwrap();
    let a1_panorama = new_state.compute_panorama(&a1_swunit).unwrap();
    new_state.add_unit(a1_swunit, a1_panorama)?;

    // With a1 it can start going through its sequence number-based panorama and find b1 missing.
    // Since it doesn't even have a unit number 0 from Bob, it requests first b0, then b1.
    let res = new_state.compute_panorama(&a2_swunit);
    assert_eq!(Err(Dependency::UnitBySeqNum(0, BOB)), res);
    let b0_swunit = state.wire_unit(&b0, TEST_INSTANCE_ID).unwrap();
    let b0_panorama = new_state.compute_panorama(&b0_swunit).unwrap();
    new_state.add_unit(b0_swunit, b0_panorama)?;
    let res = new_state.compute_panorama(&a2_swunit);
    assert_eq!(Err(Dependency::UnitBySeqNum(1, BOB)), res);
    let b1_swunit = state.wire_unit(&b1, TEST_INSTANCE_ID).unwrap();

    // Bob's b1 depends on c0. That should be all we need: Now we can add c0, then b1, then a2.
    let res = new_state.compute_panorama(&b1_swunit);
    assert_eq!(Err(Dependency::UnitBySeqNum(0, CAROL)), res);

    // But not so fast! Carol equivocated and we have the wrong unit number 0.
    let c0_prime_swunit = state.wire_unit(&c0_prime, TEST_INSTANCE_ID).unwrap();
    let c0_prime_panorama = new_state.compute_panorama(&c0_prime_swunit).unwrap();
    new_state.add_unit(c0_prime_swunit, c0_prime_panorama)?;

    // Now b1's panorama can be computed but its hash doesn't match!
    // We need to request the full hash-based panoramas. They contain the right c0 by hash.
    let res = new_state.compute_panorama(&b1_swunit);
    assert_eq!(Err(Dependency::UnitWithPanorama(b1)), res);
    let b1_panorama = state.unit(&b1).panorama.clone();
    let c0_swunit = state.wire_unit(&c0, TEST_INSTANCE_ID).unwrap();
    let c0_panorama = new_state.compute_panorama(&c0_swunit).unwrap();
    new_state.add_unit(c0_swunit, c0_panorama)?;

    // Finally we can add b1 and a2. Since a2 already knows that Carol is faulty, its panorama can
    // be computed despite the multiple forks.
    new_state.add_unit(b1_swunit, b1_panorama)?;
    let a2_panorama = new_state.compute_panorama(&a2_swunit).unwrap();
    new_state.add_unit(a2_swunit, a2_panorama)?;

    Ok(())
}

#[test]
fn ban_and_mark_faulty() -> Result<(), AddUnitError<TestContext>> {
    let params = Params::new(
        0,
        TEST_BLOCK_REWARD,
        TEST_BLOCK_REWARD / 5,
        4,
        19,
        4,
        u64::MAX,
        Timestamp::zero(),
        Timestamp::from(u64::MAX),
        TEST_ENDORSEMENT_EVIDENCE_LIMIT,
    );
    // Everyone already knows Alice is faulty, so she is banned.
    let mut state = State::new(WEIGHTS, params, vec![ALICE], vec![]);

    assert_eq!(panorama![F, N, N], *state.panorama());
    assert_eq!(Some(&Fault::Banned), state.maybe_fault(ALICE));
    let err = unit_err(add_unit!(state, ALICE, 0xA; N, N, N).err().unwrap());
    assert_eq!(UnitError::Banned, err);

    state.mark_faulty(ALICE); // No change: Banned state is permanent.
    assert_eq!(panorama![F, N, N], *state.panorama());
    assert_eq!(Some(&Fault::Banned), state.maybe_fault(ALICE));
    let err = unit_err(add_unit!(state, ALICE, 0xA; N, N, N).err().unwrap());
    assert_eq!(UnitError::Banned, err);

    // Now we also received external evidence (i.e. not in this instance) that Bob is faulty.
    state.mark_faulty(BOB);
    assert_eq!(panorama![F, F, N], *state.panorama());
    assert_eq!(Some(&Fault::Indirect), state.maybe_fault(BOB));

    // However, we still accept messages from Bob, since he is not banned.
    add_unit!(state, BOB, 0xB; F, N, N)?;
    Ok(())
}

#[test]
fn find_in_swimlane() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);
    let a0 = add_unit!(state, ALICE, 0xA; N, N, N)?;
    let mut a = vec![a0];
    for i in 1..10 {
        let ai = add_unit!(state, ALICE, None; a[i - 1], N, N)?;
        a.push(ai);
    }

    // The predecessor with sequence number i should always equal a[i].
    for j in (a.len() - 2)..a.len() {
        for i in 0..j {
            assert_eq!(Some(&a[i]), state.find_in_swimlane(&a[j], i as u64));
        }
    }

    // The skip list index of a[k] includes a[k - 2^i] for each i such that 2^i divides k.
    assert_eq!(&[a[8]], &state.unit(&a[9]).skip_idx.as_ref());
    assert_eq!(
        &[a[7], a[6], a[4], a[0]],
        &state.unit(&a[8]).skip_idx.as_ref()
    );
    Ok(())
}

#[test]
fn fork_choice() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);

    // Create blocks with scores as follows:
    //
    //          a0: 7 — a1: 3
    //        /       \
    // b0: 12           b2: 4
    //        \
    //          c0: 5 — c1: 5
    let b0 = add_unit!(state, BOB, 0xB0; N, N, N)?;
    let c0 = add_unit!(state, CAROL, 0xC0; N, b0, N)?;
    let c1 = add_unit!(state, CAROL, 0xC1; N, b0, c0)?;
    let a0 = add_unit!(state, ALICE, 0xA0; N, b0, N)?;
    let b1 = add_unit!(state, BOB, None; a0, b0, N)?; // Just a ballot; not shown above.
    let a1 = add_unit!(state, ALICE, 0xA1; a0, b1, c1)?;
    let b2 = add_unit!(state, BOB, 0xB2; a0, b1, N)?;

    // Alice built `a1` on top of `a0`, which had already 7 points.
    assert_eq!(Some(&a0), state.block(&state.unit(&a1).block).parent());
    // The fork choice is now `b2`: At height 1, `a0` wins against `c0`.
    // At height 2, `b2` wins against `a1`. `c1` has most points but is not a child of `a0`.
    assert_eq!(Some(&b2), state.fork_choice(state.panorama()));
    Ok(())
}

#[test]
fn validate_lnc_no_equivocation() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);

    // No equivocations – incoming vote doesn't violate LNC.
    // Create votes as follows; a0, b0 are blocks:
    //
    // Alice: a0 — a1
    //           /
    // Bob:   b0
    let a0 = add_unit!(state, ALICE, 0xA; N, N, N)?;
    let b0 = add_unit!(state, BOB, 0xB; N, N, N)?;

    // a1 does not violate LNC
    add_unit!(state, ALICE, None; a0, b0, N)?;
    Ok(())
}

#[test]
fn validate_lnc_fault_seen_directly() -> Result<(), AddUnitError<TestContext>> {
    // Equivocation cited by one honest validator in the vote's panorama.
    // Does NOT violate LNC.
    //
    // Bob:      b0
    //          / |
    // Alice: a0  |
    //            |
    //        a0' |
    //           \|
    // Carol:    c0
    let mut state = State::new_test(WEIGHTS, 0);

    let a0 = add_unit!(state, ALICE, 0xA; N, N, N)?;
    let b0 = add_unit!(state, BOB, 0xB; a0, N, N)?;
    let _a0_prime = add_unit!(state, ALICE, 0xA2; N, N, N)?;
    // c0 does not violate LNC b/c it sees Alice as faulty.
    add_unit!(state, CAROL, None; F, b0, N)?;
    Ok(())
}

#[test]
fn validate_lnc_one_equivocator() -> Result<(), AddUnitError<TestContext>> {
    // Equivocation cited by two honest validators in the vote's panorama – their votes need to
    // be endorsed.
    //
    // Bob:      b0
    //          / \
    // Alice: a0   \
    //              \
    //        a0'    \
    //           \   |
    // Carol:    c0  |
    //             \ |
    // Dan:         d0

    let weights4 = &[Weight(3), Weight(4), Weight(5), Weight(5)];
    let mut state = State::new_test(weights4, 0);

    let a0 = add_unit!(state, ALICE, 0xA; N, N, N, N)?;
    let a0_prime = add_unit!(state, ALICE, 0xA2; N, N, N, N)?;
    let b0 = add_unit!(state, BOB, 0xB; a0, N, N, N)?;
    let c0 = add_unit!(state, CAROL, 0xB2; a0_prime, N, N, N)?;
    // d0 violates LNC b/c it naively cites Alice's equivocation.
    // None of the votes is marked as being endorsed – violates LNC.
    assert_eq!(
        add_unit!(state, DAN, None; F, b0, c0, N).unwrap_err().cause,
        UnitError::LncNaiveCitation(ALICE)
    );
    endorse!(state, CAROL, c0);
    endorse!(state, c0; BOB, DAN);
    // Now d0 cites non-naively b/c c0 is endorsed.
    add_unit!(state, DAN, None; F, b0, c0, N; c0)?;
    Ok(())
}

#[test]
fn validate_lnc_two_equivocators() -> Result<(), AddUnitError<TestContext>> {
    // Multiple equivocators and indirect equivocations.
    // Votes are seen as endorsed by `state` – does not violate LNC.
    //
    // Alice   a0<---------+
    //                     |
    //         a0'<--+     |
    //               |     |
    // Bob          b0<-----------+
    //               |     |      |
    // Carol   c0<---+     |      |
    //                     |      |
    //         c0'<--------+      |
    //                     |      |
    // Dan                 d0<----+
    //                            |
    // Eric                       e0

    let weights5 = &[Weight(3), Weight(4), Weight(5), Weight(5), Weight(6)];
    let mut state = State::new_test(weights5, 0);

    let a0 = add_unit!(state, ALICE, 0xA; N, N, N, N, N)?;
    let a0_prime = add_unit!(state, ALICE, 0xA2; N, N, N, N, N)?;
    let c0 = add_unit!(state, CAROL, 0xC; N, N, N, N, N)?;
    let b0 = add_unit!(state, BOB, 0xB; a0_prime, N, c0, N, N)?;
    let c0_prime = add_unit!(state, CAROL, 0xC2; N, N, N, N, N)?;
    let d0 = add_unit!(state, DAN, 0xD; a0, N, c0_prime, N, N)?;
    // e0 violates LNC b/c it naively cites Alice's & Carol's equivocations.
    assert_eq!(
        add_unit!(state, ERIC, None; F, b0, F, d0, N)
            .unwrap_err()
            .cause,
        UnitError::LncNaiveCitation(ALICE)
    );
    // Endorse b0.
    endorse!(state, b0; BOB, DAN, ERIC);
    add_unit!(state,ERIC, None; F, b0, F, d0, N; b0)?;
    Ok(())
}

#[test]
fn validate_lnc_own_naive_citation() -> Result<(), AddUnitError<TestContext>> {
    //           a0'<-----+
    // Alice              |
    //           a0 <--+  |
    //                 |  |
    // Bob             |  +--b0<--+--b1
    //                 |  |       |
    // Carol           |  +--c0<--+
    //                 |          |
    // Dan             +-----d0<--+
    let weights4 = &[Weight(3), Weight(4), Weight(5), Weight(5)];
    let mut state = State::new_test(weights4, 0);

    let a0 = add_unit!(state, ALICE, 0xA; N, N, N, N)?;
    let a0_prime = add_unit!(state, ALICE, 0xA2; N, N, N, N)?;

    // Bob and Carol don't see a0 yet, so they cite a0_prime naively. Dan cites a0 naively.
    let b0 = add_unit!(state, BOB, None; a0_prime, N, N, N)?;
    let c0 = add_unit!(state, CAROL, None; a0_prime, N, N, N)?;
    let d0 = add_unit!(state, DAN, None; a0, N, N, N)?;
    endorse!(state, c0; ALICE, BOB, CAROL, DAN); // Everyone endorses c0.
    endorse!(state, d0; ALICE, BOB, CAROL, DAN); // Everyone endorses d0.

    // The fact that c0 is endorsed is not enough. Bob would violate the LNC because his new unit
    // cites a0 naively, and his previous unit b0 cited a0_prime naively.
    assert_eq!(
        add_unit!(state, BOB, None; F, b0, c0, d0; c0)
            .unwrap_err()
            .cause,
        UnitError::LncNaiveCitation(ALICE)
    );
    // The fact that d0 is endorsed makes both of Bob's units cite only one of Alice's forks
    // naively (namely a0_prime), which is fine.
    add_unit!(state, BOB, None; F, b0, c0, d0; d0)?;
    Ok(())
}

#[test]
fn validate_lnc_mixed_citations() -> Result<(), AddUnitError<TestContext>> {
    // Eric's vote should not require an endorsement as his unit e0 cites equivocator Carol before
    // the fork.
    //
    // Alice                              +a0+
    //                                    ++ |
    //                                    || |
    //                                    || |
    // Bob                     b0<---------+ |
    //                          +          | |
    //                          |          | |
    //                    +c1<--+          | |
    // Carol         c0<--+                | |
    //                ^   +c1'<-+          | |
    //                |         +          | |
    // Dan            |        d0<---------+ |
    //                |                      |
    // Eric           +--+e0<----------------+
    //
    let weights5 = &[Weight(3), Weight(4), Weight(5), Weight(5), Weight(6)];
    let mut state = State::new_test(weights5, 0);

    let c0 = add_unit!(state, CAROL, 0xC; N, N, N, N, N)?;
    let c1 = add_unit!(state, CAROL, 0xC1; N, N, c0, N, N)?;
    let c1_prime = add_unit!(state, CAROL, 0xC1B; N, N, c0, N, N)?;
    let b0 = add_unit!(state, BOB, 0xB; N, N, c1, N, N)?;
    let d0 = add_unit!(state, DAN, 0xD; N, N, c1_prime, N, N)?;
    // Should not require endorsements b/c e0 sees Carol as correct.
    let e0 = add_unit!(state, ERIC, 0xE; N, N, c0, N, N)?;
    assert_eq!(
        add_unit!(state, ALICE, None; N, b0, F, d0, e0)
            .unwrap_err()
            .cause,
        UnitError::LncNaiveCitation(CAROL)
    );
    // We pick b0 to be endorsed.
    endorse!(state, b0; ALICE, BOB, ERIC);
    add_unit!(state, ALICE, None; N, b0, F, d0, e0; b0)?;
    Ok(())
}

#[test]
fn validate_lnc_transitive_endorsement() -> Result<(), AddUnitError<TestContext>> {
    // Endorsements should be transitive to descendants.
    // c1 doesn't have to be endorsed, it is enough that c0 is.
    //
    // Alice           a0<-----------+
    //                 +             |
    //          b0<----+             |
    // Bob                           |
    //                               |
    //          b0'<---+             |
    //                 +             |
    // Carol           c0<---+c1<----+
    //                               |
    //                               |
    // Dan                          d0

    let weights_dan = &[Weight(3), Weight(4), Weight(5), Weight(5)];
    let mut state = State::new_test(weights_dan, 0);

    let b0 = add_unit!(state, BOB, 0xB; N, N, N, N)?;
    let b0_prime = add_unit!(state, BOB, 0xB1; N, N, N, N)?;
    let a0 = add_unit!(state, ALICE, 0xA; N, b0, N, N)?;
    let c0 = add_unit!(state, CAROL, 0xC; N, b0_prime, N, N)?;
    let c1 = add_unit!(state, CAROL, 0xC1; N, b0_prime, c0, N)?;
    assert_eq!(
        add_unit!(state, DAN, None; a0, F, c1, N).unwrap_err().cause,
        UnitError::LncNaiveCitation(BOB)
    );
    endorse!(state, c0; CAROL, DAN, ALICE);
    add_unit!(state, DAN, None; a0, F, c1, N; c0)?;
    Ok(())
}

#[test]
fn validate_lnc_cite_descendant_of_equivocation() -> Result<(), AddUnitError<TestContext>> {
    // a0 cites a descendant b1 of an equivocation vote (b0 and b0').
    // This is still detected as violation of the LNC.
    //
    // Alice                  a0<----+
    //                        +      |
    //          b0<---+b1<----+      |
    // Bob                           |
    //                               |
    //          b0'<---+             |
    //                 +             |
    // Carol           c0            |
    //                  ^            +
    // Dan              +----------+d0
    let weights_dan = &[Weight(3), Weight(4), Weight(5), Weight(5)];
    let mut state = State::new_test(weights_dan, 0);

    let b0 = add_unit!(state, BOB, 0xB; N, N, N, N)?;
    let b0_prime = add_unit!(state, BOB, 0xBA; N, N, N, N)?;
    let b1 = add_unit!(state, BOB, 0xB1; N, b0, N, N)?;
    let a0 = add_unit!(state, ALICE, 0xA; N, b1, N, N)?;
    let c0 = add_unit!(state, CAROL, 0xC; N, b0_prime, N, N)?;
    assert_eq!(
        add_unit!(state, DAN, None; a0, F, c0, N).unwrap_err().cause,
        UnitError::LncNaiveCitation(BOB)
    );
    endorse!(state, c0; ALICE, CAROL, DAN);
    add_unit!(state, DAN, None; a0, F, c0, N; c0)?;
    Ok(())
}

#[test]
fn validate_lnc_endorse_mix_pairs() -> Result<(), AddUnitError<TestContext>> {
    // Diagram of the DAG can be found under
    // /resources/test/dags/validate_lnc_endorse_mix_pairs.png
    //
    // Both c0 and g0 need only one of their descendants votes endorsed to not validate LNC.
    // Since endorsements are monotonic (c0's and g0' endorsements are also endorsed by h0),
    // h0 does not violate LNC b/c it cites at most one fork naively.
    let weights = &[
        Weight(3),
        Weight(4),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
    ];
    let mut state = State::new_test(weights, 0);

    let d0 = add_unit!(state, DAN, 0xBA; N, N, N, N, N, N, N, N)?;
    let d0_prime = add_unit!(state, DAN, 0xBB; N, N, N, N, N, N, N, N)?;
    let a0 = add_unit!(state, ALICE, None; N, N, N, d0, N, N, N, N)?;
    let b0 = add_unit!(state, BOB, None; N, N, N, d0_prime, N, N, N, N)?;
    endorse!(state, a0; ALICE, BOB, CAROL, ERIC, FRANK, GINA);
    let c0 = add_unit!(state, CAROL, None; a0, b0, N, F, N, N, N, N; a0)?;
    let e0 = add_unit!(state, ERIC, None; N, N, N, d0, N, N, N, N)?;
    let f0 = add_unit!(state, FRANK, None; N, N, N, d0_prime, N, N, N, N)?;
    endorse!(state, f0; ALICE, BOB, CAROL, ERIC, FRANK, GINA);
    let g0 = add_unit!(state, GINA, None; N, N, N, F, e0, f0, N, N; f0)?;
    // Should pass the LNC validation test.
    add_unit!(state, HANNA, None; a0, b0, c0, F, e0, f0, g0, N; a0, f0)?;
    Ok(())
}

#[test]
fn validate_lnc_shared_equiv_unit() -> Result<(), AddUnitError<TestContext>> {
    // Diagram of the DAG can be found under
    // /resources/test/dags/validate_lnc_shared_equiv_unit.png
    let weights = &[
        Weight(3),
        Weight(4),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
    ];
    let mut state = State::new_test(weights, 0);

    let d0 = add_unit!(state, DAN, 0xDA; N, N, N, N, N, N, N, N)?;
    let d0_prime = add_unit!(state, DAN, 0xDB; N, N, N, N, N, N, N, N)?;
    let d0_bis = add_unit!(state, DAN, 0xDC; N, N, N, N, N, N, N, N)?;
    let b0 = add_unit!(state, BOB, None; N, N, N, d0, N, N, N, N)?;
    let c0 = add_unit!(state, CAROL, None; N, N, N, d0_prime, N, N, N, N)?;
    let e0 = add_unit!(state, ERIC, None; N, N, N, d0_prime, N, N, N, N)?;
    let f0 = add_unit!(state, FRANK, None; N, N, N, d0_bis, N, N, N, N)?;
    endorse!(state, c0; ALICE, BOB, CAROL, ERIC, FRANK);
    let a0 = add_unit!(state, ALICE, None; N, b0, c0, F, N, N, N, N; c0)?;
    endorse!(state, e0; ALICE, BOB, CAROL, ERIC, FRANK);
    let g0 = add_unit!(state, GINA, None; N, N, N, F, e0, f0, N, N; e0)?;
    assert_eq!(
        add_unit!(state, HANNA, None; a0, b0, c0, F, e0, f0, g0, N; c0, e0)
            .unwrap_err()
            .cause,
        UnitError::LncNaiveCitation(DAN)
    );
    // Even though both a0 and g0 cite DAN non-naively, for h0 to be valid
    // we need to endorse either b0 or f0.
    let mut pre_endorse_state = state.clone();
    // Endorse b0 first.
    endorse!(state, b0; ALICE, BOB, CAROL, ERIC, FRANK);
    add_unit!(state, HANNA, None; a0, b0, c0, F, e0, f0, g0, N; c0, e0, b0)?;
    // Should also pass if e0 is endorsed.
    endorse!(pre_endorse_state, f0; ALICE, BOB, CAROL, ERIC, FRANK);
    add_unit!(pre_endorse_state, HANNA, None; a0, b0, c0, F, e0, f0, g0, N; c0, e0, f0)?;
    Ok(())
}

#[test]
fn validate_lnc_four_forks() -> Result<(), AddUnitError<TestContext>> {
    // Diagram of the DAG can be found under
    // /resources/test/dags/validate_lnc_four_forks.png
    let weights = &[
        Weight(3),
        Weight(4),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
        Weight(5),
    ];
    let mut state = State::new_test(weights, 0);

    let e0 = add_unit!(state, ERIC, 0xEA; N, N, N, N, N, N, N, N)?;
    let e0_prime = add_unit!(state, ERIC, 0xEB; N, N, N, N, N, N, N, N)?;
    let e0_bis = add_unit!(state, ERIC, 0xEC; N, N, N, N, N, N, N, N)?;
    let e0_cis = add_unit!(state, ERIC, 0xED; N, N, N, N, N, N, N, N)?;
    let a0 = add_unit!(state, ALICE, None; N, N, N, N, e0, N, N, N)?;
    let b0 = add_unit!(state, BOB, None; N, N, N, N, e0_prime, N, N, N)?;
    let g0 = add_unit!(state, GINA, None; N, N, N, N, e0_bis, N, N, N)?;
    let h0 = add_unit!(state, HANNA, None; N, N, N, N, e0_cis, N, N, N)?;
    endorse!(state, a0; ALICE, BOB, CAROL, DAN, GINA, HANNA);
    let c0 = add_unit!(state, CAROL, None; a0, b0, N, N, F, N, N, N; a0)?;
    endorse!(state, g0; ALICE, BOB, CAROL, DAN, GINA, HANNA);
    let f0 = add_unit!(state, FRANK, None; N, N, N, N, F, N, g0, h0; g0)?;
    let d0 = add_unit!(state, DAN, None; N, N, N, N, F, f0, g0, h0; g0)?;
    assert_eq!(
        add_unit!(state, DAN, None; a0, b0, c0, d0, F, f0, g0, h0; a0, g0)
            .unwrap_err()
            .cause,
        UnitError::LncNaiveCitation(ERIC)
    );
    let mut pre_endorse_state = state.clone();
    // If we endorse h0, then d1 still violates the LNC: d0 cited e0_cis naively and d1 cites
    // e0_prime naively.
    endorse!(state, h0; ALICE, BOB, CAROL, DAN, GINA, HANNA);
    let result = add_unit!(state, DAN, None; a0, b0, c0, d0, F, f0, g0, h0; a0, g0, h0);
    assert_eq!(result.unwrap_err().cause, UnitError::LncNaiveCitation(ERIC));
    // It should work if we had endorsed b0 instead.
    endorse!(pre_endorse_state, b0; ALICE, BOB, CAROL, DAN, GINA, HANNA);
    add_unit!(pre_endorse_state, DAN, None; a0, b0, c0, d0, F, f0, g0, h0; a0, g0, b0)?;
    // And it should still work if both were endorsed.
    endorse!(pre_endorse_state, h0; ALICE, BOB, CAROL, DAN, GINA, HANNA);
    add_unit!(pre_endorse_state, DAN, None; a0, b0, c0, d0, F, f0, g0, h0; a0, g0, b0, h0)?;
    Ok(())
}

#[test]
fn is_terminal_block() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);

    let a0 = add_unit!(state, ALICE, 0x00; N, N, N)?;
    assert!(!state.is_terminal_block(&a0)); // height 0
    let b0 = add_unit!(state, BOB, 0x01; a0, N, N)?;
    assert!(!state.is_terminal_block(&b0)); // height 1
    let c0 = add_unit!(state, CAROL, 0x02; a0, b0, N)?;
    assert!(!state.is_terminal_block(&c0)); // height 2
    let a1 = add_unit!(state, ALICE, 0x03; a0, b0, c0)?;
    assert!(!state.is_terminal_block(&a1)); // height 3
    let a2 = add_unit!(state, ALICE, None; a1, b0, c0)?;
    assert!(!state.is_terminal_block(&a2)); // not a block
    let a3 = add_unit!(state, ALICE, 0x04; a2, b0, c0)?;
    assert!(state.is_terminal_block(&a3)); // height 4, i.e. the fifth block and thus the last one
    assert_eq!(TEST_ERA_HEIGHT - 1, state.block(&a3).height);
    let a4 = add_unit!(state, ALICE, None; a3, b0, c0)?;
    assert!(!state.is_terminal_block(&a4)); // not a block
    Ok(())
}

#[test]
fn conflicting_endorsements() -> Result<(), AddUnitError<TestContext>> {
    if TODO_ENDORSEMENT_EVIDENCE_DISABLED {
        return Ok(()); // Endorsement evidence is disabled, so don't test it.
    }
    let validators = vec![(ALICE_SEC, ALICE), (BOB_SEC, BOB), (CAROL_SEC, CAROL)]
        .into_iter()
        .map(|(sk, vid)| {
            assert_eq!(sk.0, vid.0);
            (sk.0, WEIGHTS[vid.0 as usize].0)
        })
        .collect();
    let mut state = State::new_test(WEIGHTS, 0);

    // Alice equivocates and creates two forks:
    // * a0_prime and
    // * a0 < a1 < a2
    let a0 = add_unit!(state, ALICE, 0x00; N, N, N)?;
    let a0_prime = add_unit!(state, ALICE, 0x01; N, N, N)?;
    let a1 = add_unit!(state, ALICE, None; a0, N, N)?;
    let a2 = add_unit!(state, ALICE, None; a1, N, N)?;

    // Bob endorses a0_prime.
    endorse!(state, BOB, a0_prime);
    assert!(!state.is_faulty(BOB));

    // Now he also endorses a2, even though it is on a different fork. That's a fault!
    endorse!(state, BOB, a2);
    assert!(state.is_faulty(BOB));

    let evidence = state
        .maybe_evidence(BOB)
        .expect("Bob should be considered faulty")
        .clone();
    assert_eq!(
        Ok(()),
        evidence.validate(&validators, &TEST_INSTANCE_ID, state.params())
    );

    let limit = TEST_ENDORSEMENT_EVIDENCE_LIMIT as usize;

    let mut a = vec![a0, a1, a2];
    while a.len() <= limit + 1 {
        let prev_a = *a.last().unwrap();
        a.push(add_unit!(state, ALICE, None; prev_a, N, N)?);
    }

    // Carol endorses a0_prime.
    endorse!(state, CAROL, a0_prime);

    // She also endorses a[limit + 1], and gets away with it because the evidence would be too big.
    endorse!(state, CAROL, a[limit + 1]);
    assert!(!state.is_faulty(CAROL));

    // But if she endorses a[limit], the units fit into an evidence vertex.
    endorse!(state, CAROL, a[limit]);
    assert!(state.is_faulty(CAROL));

    let evidence = state
        .maybe_evidence(CAROL)
        .expect("Carol is faulty")
        .clone();
    assert_eq!(
        Ok(()),
        evidence.validate(&validators, &TEST_INSTANCE_ID, state.params())
    );

    // If the limit were less, that evidence would be considered invalid.
    let params2 = test_params(0).with_endorsement_evidence_limit(limit as u64 - 1);
    assert_eq!(
        Err(EvidenceError::EndorsementTooManyUnits),
        evidence.validate(&validators, &TEST_INSTANCE_ID, &params2)
    );

    Ok(())
}

#[test]
fn retain_evidence_only() -> Result<(), AddUnitError<TestContext>> {
    let mut state = State::new_test(WEIGHTS, 0);

    let a0 = add_unit!(state, ALICE, 0xA; N, N, N)?;
    let b0 = add_unit!(state, BOB, 0xB; a0, N, N)?;
    let _a0_prime = add_unit!(state, ALICE, 0xA2; N, N, N)?;
    assert_eq!(&panorama!(F, b0, N), state.panorama());
    state.retain_evidence_only();
    assert_eq!(&panorama!(F, N, N), state.panorama());
    assert!(!state.has_unit(&a0));
    assert!(state.has_evidence(ALICE));
    Ok(())
}

#[test]
fn test_log2() {
    assert_eq!(0, log2(0));
    assert_eq!(2, log2(0b100));
    assert_eq!(2, log2(0b101));
    assert_eq!(2, log2(0b111));
    assert_eq!(3, log2(0b1000));
    assert_eq!(63, log2(u64::MAX));
    assert_eq!(63, log2(1 << 63));
    assert_eq!(62, log2((1 << 63) - 1));
}

#[test]
fn test_leader() {
    let weights = &[Weight(3), Weight(4), Weight(5), Weight(4), Weight(5)];

    // All five validators get slots in the leader sequence. If 1, 2 and 4 are excluded, their slots
    // get reassigned, but 0 and 3 keep their old slots.
    let before = vec![0, 2, 4, 3, 3, 1, 2, 1, 0, 0, 0, 2, 0, 2, 3, 2, 3, 3, 1, 2];
    let after = vec![0, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 3, 3, 3, 3];
    let excluded = vec![ValidatorIndex(1), ValidatorIndex(2), ValidatorIndex(4)];
    let state = State::<TestContext>::new(weights, test_params(0), vec![], vec![]);
    assert_eq!(
        before,
        (0..20u64)
            .map(|r_id| state.leader(r_id.into()).0)
            .collect_vec()
    );
    let state = State::<TestContext>::new(weights, test_params(0), vec![], excluded);
    assert_eq!(
        after,
        (0..20u64)
            .map(|r_id| state.leader(r_id.into()).0)
            .collect_vec()
    );
}

#[test]
fn test_leader_prng() {
    let mut rng = crate::new_rng();

    // Repeat a few times to make it likely that the inner loop runs more than once.
    for _ in 0..10 {
        let upper = rng.gen_range(1..u64::MAX);
        let seed = rng.next_u64();

        // This tests that the rand crate's gen_range implementation, which is used in
        // leader_prng, doesn't change, and uses this algorithm:
        // https://github.com/rust-random/rand/blob/73befa480c58dd0461da5f4469d5e04c564d4de3/src/distributions/uniform.rs#L515
        let mut prng = ChaCha8Rng::seed_from_u64(seed);
        let zone = upper << upper.leading_zeros(); // A multiple of upper that fits into a u64.
        let expected = loop {
            // Multiply a random u64 by upper. This is between 0 and u64::MAX * upper.
            let prod = (prng.next_u64() as u128) * (upper as u128);
            // So prod >> 64 is between 0 and upper - 1. Each interval from (N << 64) to
            // (N << 64) + zone contains the same number of such values.
            // If the value is in such an interval, return N + 1; otherwise retry.
            if (prod as u64) < zone {
                break (prod >> 64) as u64 + 1;
            }
        };

        assert_eq!(expected, leader_prng(upper, seed));
    }
}

#[test]
fn test_leader_prng_values() {
    // Test a few concrete values, to detect if the ChaCha8Rng impl changes.
    assert_eq!(12578764544318200737, leader_prng(u64::MAX, 42));
    assert_eq!(12358540700710939054, leader_prng(u64::MAX, 1337));
    assert_eq!(4134160578770126600, leader_prng(u64::MAX, 0x1020304050607));
}
