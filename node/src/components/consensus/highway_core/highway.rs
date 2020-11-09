mod vertex;

pub(crate) use crate::components::consensus::highway_core::state::Params;
pub(crate) use vertex::{Dependency, Endorsements, SignedWireVote, Vertex, WireVote};

use thiserror::Error;
use tracing::{debug, error, info};

use crate::{
    components::consensus::{
        consensus_protocol::BlockContext,
        highway_core::{
            active_validator::{ActiveValidator, Effect},
            evidence::EvidenceError,
            state::{Fault, State, VoteError},
            validators::{Validator, Validators},
        },
        traits::Context,
    },
    types::{CryptoRngCore, Timestamp},
};

use super::{
    endorsement::{Endorsement, EndorsementError},
    evidence::Evidence,
};

/// An error due to an invalid vertex.
#[derive(Debug, Error, PartialEq)]
pub(crate) enum VertexError {
    #[error("The vertex contains an invalid vote: `{0}`")]
    Vote(#[from] VoteError),
    #[error("The vertex contains invalid evidence.")]
    Evidence(#[from] EvidenceError),
    #[error("The endorsements contains invalid entry.")]
    Endorsement(#[from] EndorsementError),
}

/// A vertex that has passed initial validation.
///
/// The vertex could not be determined to be invalid based on its contents alone. The remaining
/// checks will be applied once all of its dependencies have been added to `Highway`. (See
/// `ValidVertex`.)
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreValidatedVertex<C: Context>(Vertex<C>);

impl<C: Context> PreValidatedVertex<C> {
    pub(crate) fn inner(&self) -> &Vertex<C> {
        &self.0
    }

    pub(crate) fn timestamp(&self) -> Option<Timestamp> {
        self.0.timestamp()
    }

    #[cfg(test)]
    pub(crate) fn into_vertex(self) -> Vertex<C> {
        self.0
    }
}

impl<C: Context> From<ValidVertex<C>> for PreValidatedVertex<C> {
    fn from(vv: ValidVertex<C>) -> PreValidatedVertex<C> {
        PreValidatedVertex(vv.0)
    }
}

impl<C: Context> From<ValidVertex<C>> for Vertex<C> {
    fn from(vv: ValidVertex<C>) -> Vertex<C> {
        vv.0
    }
}

impl<C: Context> From<PreValidatedVertex<C>> for Vertex<C> {
    fn from(pvv: PreValidatedVertex<C>) -> Vertex<C> {
        pvv.0
    }
}

/// A vertex that has been validated: `Highway` has all its dependencies and can add it to its
/// protocol state.
///
/// Note that this must only be added to the `Highway` instance that created it. Can cause a panic
/// or inconsistent state otherwise.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidVertex<C: Context>(pub(super) Vertex<C>);

impl<C: Context> ValidVertex<C> {
    pub(crate) fn inner(&self) -> &Vertex<C> {
        &self.0
    }

    pub(crate) fn is_proposal(&self) -> bool {
        self.0.value().is_some()
    }

    pub(crate) fn endorsements(&self) -> Option<&Endorsements<C>> {
        match &self.0 {
            Vertex::Endorsements(endorsements) => Some(endorsements),
            Vertex::Evidence(_) | Vertex::Vote(_) => None,
        }
    }
}

/// A result indicating whether and how a requested dependency is satisfied.
pub(crate) enum GetDepOutcome<C: Context> {
    /// We don't have this dependency.
    None,
    /// This vertex satisfies the dependency.
    Vertex(ValidVertex<C>),
    /// The dependency must be satisfied by providing evidence against this faulty validator, but
    /// this `Highway` instance does not have direct evidence.
    Evidence(C::ValidatorId),
}

/// A passive instance of the Highway protocol, containing its local state.
///
/// Both observers and active validators must instantiate this, pass in all incoming vertices from
/// peers, and use a [FinalityDetector](../finality_detector/struct.FinalityDetector.html) to
/// determine the outcome of the consensus process.
#[derive(Debug)]
pub(crate) struct Highway<C: Context> {
    /// The protocol instance ID. This needs to be unique, to prevent replay attacks.
    instance_id: C::InstanceId,
    /// The validator IDs and weight map.
    validators: Validators<C::ValidatorId>,
    /// The abstract protocol state.
    state: State<C>,
    /// The state of an active validator, who is participating and creating new vertices.
    active_validator: Option<ActiveValidator<C>>,
}

impl<C: Context> Highway<C> {
    /// Creates a new `Highway` instance. All participants must agree on the protocol parameters.
    ///
    /// Arguments:
    ///
    /// * `instance_id`: A unique identifier for every execution of the protocol (e.g. for every
    ///   era) to prevent replay attacks.
    /// * `validators`: The set of validators and their weights.
    /// * `params`: The Highway protocol parameters.
    pub(crate) fn new(
        instance_id: C::InstanceId,
        validators: Validators<C::ValidatorId>,
        params: Params,
    ) -> Highway<C> {
        info!(%validators, "creating Highway instance {:?}", instance_id);
        let weights = validators.iter().map(Validator::weight);
        let banned = validators.iter_banned_idx();
        let state = State::new(weights, params, banned);
        Highway {
            instance_id,
            validators,
            state,
            active_validator: None,
        }
    }

    /// Turns this instance from a passive observer into an active validator that proposes new
    /// blocks and creates and signs new vertices.
    ///
    /// Panics if `id` is not the ID of a validator with a weight in this Highway instance.
    pub(crate) fn activate_validator(
        &mut self,
        id: C::ValidatorId,
        secret: C::ValidatorSecret,
        current_time: Timestamp,
    ) -> Vec<Effect<C>> {
        assert!(
            self.active_validator.is_none(),
            "activate_validator called twice"
        );
        let idx = self
            .validators
            .get_index(&id)
            .expect("missing own validator ID");
        let start_time = current_time.max(self.state.params().start_timestamp());
        let (av, effects) = ActiveValidator::new(idx, secret, start_time, &self.state);
        self.active_validator = Some(av);
        effects
    }

    /// Turns this instance into a passive observer, that does not create any new vertices.
    pub(crate) fn deactivate_validator(&mut self) {
        self.active_validator = None;
    }

    /// Switches the active validator to a new round exponent.
    pub(crate) fn set_round_exp(&mut self, new_round_exp: u8) {
        if let Some(ref mut av) = self.active_validator {
            av.set_round_exp(new_round_exp);
        }
    }

    /// Does initial validation. Returns an error if the vertex is invalid.
    pub(crate) fn pre_validate_vertex(
        &self,
        vertex: Vertex<C>,
    ) -> Result<PreValidatedVertex<C>, (Vertex<C>, VertexError)> {
        match self.do_pre_validate_vertex(&vertex) {
            Err(err) => Err((vertex, err)),
            Ok(()) => Ok(PreValidatedVertex(vertex)),
        }
    }

    /// Returns the next missing dependency, or `None` if all dependencies of `pvv` are satisfied.
    ///
    /// If this returns `None`, `validate_vertex` can be called.
    pub(crate) fn missing_dependency(&self, pvv: &PreValidatedVertex<C>) -> Option<Dependency<C>> {
        match pvv.inner() {
            Vertex::Evidence(_) => None,
            Vertex::Endorsements(endorsements) => {
                let vote = *endorsements.vote();
                if !self.state.has_vote(&vote) {
                    Some(Dependency::Vote(vote))
                } else {
                    None
                }
            }
            Vertex::Vote(vote) => vote
                .wire_vote
                .panorama
                .missing_dependency(&self.state)
                .or_else(|| {
                    self.state
                        .needs_endorsements(vote)
                        .map(Dependency::Endorsement)
                }),
        }
    }

    /// Does full validation. Returns an error if the vertex is invalid.
    ///
    /// All dependencies must be added to the state before this validation step.
    pub(crate) fn validate_vertex(
        &self,
        pvv: PreValidatedVertex<C>,
    ) -> Result<ValidVertex<C>, (PreValidatedVertex<C>, VertexError)> {
        match self.do_validate_vertex(pvv.inner()) {
            Err(err) => Err((pvv, err)),
            Ok(()) => Ok(ValidVertex(pvv.0)),
        }
    }

    /// Add a validated vertex to the protocol state.
    ///
    /// The validation must have been performed by _this_ `Highway` instance.
    /// More precisely: The instance on which `add_valid_vertex` is called must contain everything
    /// (and possibly more) that the instance on which `validate_vertex` was called contained.
    pub(crate) fn add_valid_vertex(
        &mut self,
        ValidVertex(vertex): ValidVertex<C>,
        rng: &mut dyn CryptoRngCore,
        now: Timestamp,
    ) -> Vec<Effect<C>> {
        if !self.has_vertex(&vertex) {
            match vertex {
                Vertex::Vote(vote) => self.add_valid_vote(vote, now, rng),
                Vertex::Evidence(evidence) => self.add_evidence(evidence, rng),
                Vertex::Endorsements(endorsements) => {
                    self.state.add_endorsements(endorsements);
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }

    /// Returns whether the vertex is already part of this protocol state.
    pub(crate) fn has_vertex(&self, vertex: &Vertex<C>) -> bool {
        match vertex {
            Vertex::Vote(vote) => self.state.has_vote(&vote.hash()),
            Vertex::Evidence(evidence) => self.state.has_evidence(evidence.perpetrator()),
            Vertex::Endorsements(endorsements) => {
                let vote = endorsements.vote();
                self.state.is_endorsed(vote)
                    || self
                        .state
                        .has_all_endorsements(vote, endorsements.validator_ids())
            }
        }
    }

    /// Returns whether the validator is known to be faulty and we have evidence.
    pub(crate) fn has_evidence(&self, vid: &C::ValidatorId) -> bool {
        self.validators
            .get_index(vid)
            .map_or(false, |vidx| self.state.has_evidence(vidx))
    }

    /// Marks the given validator as faulty, if it exists.
    pub(crate) fn mark_faulty(&mut self, vid: &C::ValidatorId) {
        if let Some(vidx) = self.validators.get_index(vid) {
            self.state.mark_faulty(vidx);
        }
    }

    /// Returns whether we have a vertex that satisfies the dependency.
    pub(crate) fn has_dependency(&self, dependency: &Dependency<C>) -> bool {
        match dependency {
            Dependency::Vote(hash) => self.state.has_vote(hash),
            Dependency::Evidence(idx) => self.state.is_faulty(*idx),
            Dependency::Endorsement(hash) => self.state.is_endorsed(hash),
        }
    }

    /// Returns a vertex that satisfies the dependency, if available.
    ///
    /// If we send a vertex to a peer who is missing a dependency, they will ask us for it. In that
    /// case, `get_dependency` will never return `None`, unless the peer is faulty.
    pub(crate) fn get_dependency(&self, dependency: &Dependency<C>) -> GetDepOutcome<C> {
        match dependency {
            Dependency::Vote(hash) => match self.state.wire_vote(hash, self.instance_id) {
                None => GetDepOutcome::None,
                Some(vote) => GetDepOutcome::Vertex(ValidVertex(Vertex::Vote(vote))),
            },
            Dependency::Evidence(idx) => match self.state.opt_fault(*idx) {
                None | Some(Fault::Banned) => GetDepOutcome::None,
                Some(Fault::Direct(ev)) => {
                    GetDepOutcome::Vertex(ValidVertex(Vertex::Evidence(ev.clone())))
                }
                Some(Fault::Indirect) => {
                    let vid = self.validators.id(*idx).expect("missing validator").clone();
                    GetDepOutcome::Evidence(vid)
                }
            },
            Dependency::Endorsement(hash) => match self.state.opt_endorsements(hash) {
                None => GetDepOutcome::None,
                Some(e) => {
                    GetDepOutcome::Vertex(ValidVertex(Vertex::Endorsements(Endorsements::new(e))))
                }
            },
        }
    }

    pub(crate) fn handle_timer(
        &mut self,
        timestamp: Timestamp,
        rng: &mut dyn CryptoRngCore,
    ) -> Vec<Effect<C>> {
        let instance_id = self.instance_id;

        // Here we just use the timer's timestamp, and assume it's ~ Timestamp::now()
        //
        // This is because proposal votes, i.e. new blocks, are
        // supposed to thave the exact timestamp that matches the
        // beginning of the round (which we use as the "round ID").
        //
        // But at least any discrepancy here can only come from event
        // handling delays in our own node, and not from timestamps
        // set by other nodes.

        self.map_active_validator(
            |av, state, rng| av.handle_timer(timestamp, state, instance_id, rng),
            timestamp,
            rng,
        )
        .unwrap_or_else(|| {
            debug!(%timestamp, "Ignoring `handle_timer` event: only an observer node.");
            vec![]
        })
    }

    pub(crate) fn propose(
        &mut self,
        value: C::ConsensusValue,
        block_context: BlockContext,
        rng: &mut dyn CryptoRngCore,
    ) -> Vec<Effect<C>> {
        let instance_id = self.instance_id;

        // We just use the block context's timestamp, which is
        // hopefully not much older than `Timestamp::now()`
        //
        // We do this because essentially what happens is this:
        //
        // 1. We realize it's our turn to propose a block in
        // millisecond 64, so we set a timer.
        //
        // 2. The timer for timestamp 64 fires, and we request deploys
        // for the new block from the block proposer (with 64 in the
        // block context).
        //
        // 3. The block proposer responds and we finally end up here,
        // and can propose the new block. But we still have to use
        // timestamp 64.

        let timestamp = block_context.timestamp();
        self.map_active_validator(
            |av, state, rng| av.propose(value, block_context, state, instance_id, rng),
            timestamp,
            rng,
        )
        .unwrap_or_else(|| {
            debug!("ignoring `propose` event: validator has been deactivated");
            vec![]
        })
    }

    pub(crate) fn validators(&self) -> &Validators<C::ValidatorId> {
        &self.validators
    }

    /// Returns an iterator over all validators against which we have direct evidence.
    pub(crate) fn validators_with_evidence(&self) -> impl Iterator<Item = &C::ValidatorId> {
        self.validators
            .iter()
            .enumerate()
            .filter(move |(i, _)| self.state.has_evidence((*i as u32).into()))
            .map(|(_, v)| v.id())
    }

    pub(crate) fn state(&self) -> &State<C> {
        &self.state
    }

    fn on_new_vote(
        &mut self,
        vhash: &C::Hash,
        timestamp: Timestamp,
        rng: &mut dyn CryptoRngCore,
    ) -> Vec<Effect<C>> {
        let instance_id = self.instance_id;
        self.map_active_validator(
            |av, state, rng| av.on_new_vote(vhash, timestamp, state, instance_id, rng),
            timestamp,
            rng,
        )
        .unwrap_or_default()
    }

    /// Takes action on a new evidence.
    fn on_new_evidence(
        &mut self,
        evidence: Evidence<C>,
        rng: &mut dyn CryptoRngCore,
    ) -> Vec<Effect<C>> {
        let state = &self.state;
        let mut effects = self
            .active_validator
            .as_mut()
            .map(|av| av.on_new_evidence(&evidence, state, rng))
            .unwrap_or_default();
        // Add newly created endorsements to the local state.
        for effect in effects.iter() {
            if let Effect::NewVertex(vv) = effect {
                if let Some(e) = vv.endorsements() {
                    self.state.add_endorsements(e.clone());
                }
            }
        }
        // Gossip `Evidence` only if we just learned about faults by the validator.
        effects.extend(vec![Effect::NewVertex(ValidVertex(Vertex::Evidence(
            evidence,
        )))]);
        effects
    }

    /// Applies `f` if this is an active validator, otherwise returns `None`.
    ///
    /// Newly created vertices are added to the state. If an equivocation of this validator is
    /// detected, it gets deactivated.
    fn map_active_validator<F>(
        &mut self,
        f: F,
        timestamp: Timestamp,
        rng: &mut dyn CryptoRngCore,
    ) -> Option<Vec<Effect<C>>>
    where
        F: FnOnce(&mut ActiveValidator<C>, &State<C>, &mut dyn CryptoRngCore) -> Vec<Effect<C>>,
    {
        let effects = f(self.active_validator.as_mut()?, &self.state, rng);
        let mut result = vec![];
        for effect in &effects {
            match effect {
                Effect::NewVertex(vv) => {
                    result.extend(self.add_valid_vertex(vv.clone(), rng, timestamp))
                }
                Effect::WeEquivocated(_) => self.deactivate_validator(),
                Effect::ScheduleTimer(_) | Effect::RequestNewBlock(_) => (),
            }
        }
        result.extend(effects);
        Some(result)
    }

    /// Performs initial validation and returns an error if `vertex` is invalid. (See
    /// `PreValidatedVertex` and `validate_vertex`.)
    fn do_pre_validate_vertex(&self, vertex: &Vertex<C>) -> Result<(), VertexError> {
        match vertex {
            Vertex::Vote(vote) => {
                let creator = vote.wire_vote.creator;
                let v_id = self.validators.id(creator).ok_or(VoteError::Creator)?;
                if vote.wire_vote.instance_id != self.instance_id {
                    return Err(VoteError::InstanceId.into());
                }
                if !C::verify_signature(&vote.hash(), v_id, &vote.signature) {
                    return Err(VoteError::Signature.into());
                }
                Ok(self.state.pre_validate_vote(vote)?)
            }
            Vertex::Evidence(evidence) => {
                let v_id = self
                    .validators
                    .id(evidence.perpetrator())
                    .ok_or(EvidenceError::UnknownPerpetrator)?;
                Ok(evidence.validate(v_id, &self.instance_id)?)
            }
            Vertex::Endorsements(endorsements) => {
                let vote = *endorsements.vote();
                for (v_id, signature) in endorsements.endorsers.iter() {
                    let validator = self.validators.id(*v_id).ok_or(EndorsementError::Creator)?;
                    let endorsement: Endorsement<C> = Endorsement::new(vote, *v_id);
                    if !C::verify_signature(&endorsement.hash(), validator, &signature) {
                        return Err(EndorsementError::Signature.into());
                    }
                }
                Ok(())
            }
        }
    }

    /// Validates `vertex` and returns an error if it is invalid.
    /// This requires all dependencies to be present.
    fn do_validate_vertex(&self, vertex: &Vertex<C>) -> Result<(), VertexError> {
        match vertex {
            Vertex::Vote(vote) => Ok(self.state.validate_vote(vote)?),
            Vertex::Evidence(_evidence) => Ok(()),
            Vertex::Endorsements(_endorsements) => {
                // TODO: Validate against equivocations in endorsements.
                Ok(())
            }
        }
    }

    /// Adds evidence to the protocol state.
    /// Gossip the evidence if it's the first equivocation from the creator.
    fn add_evidence(
        &mut self,
        evidence: Evidence<C>,
        rng: &mut dyn CryptoRngCore,
    ) -> Vec<Effect<C>> {
        if self.state.add_evidence(evidence.clone()) {
            self.on_new_evidence(evidence, rng)
        } else {
            vec![]
        }
    }

    /// Adds a valid vote to the protocol state.
    ///
    /// Validity must be checked before calling this! Adding an invalid vote will result in a panic
    /// or an inconsistent state.
    fn add_valid_vote(
        &mut self,
        swvote: SignedWireVote<C>,
        now: Timestamp,
        rng: &mut dyn CryptoRngCore,
    ) -> Vec<Effect<C>> {
        let vote_hash = swvote.hash();
        let creator = swvote.wire_vote.creator;
        let was_honest = !self.state.is_faulty(creator);
        self.state.add_valid_vote(swvote);
        let mut evidence_effects = self
            .state
            .opt_evidence(creator)
            .cloned()
            .map(|ev| {
                if was_honest {
                    self.on_new_evidence(ev, rng)
                } else {
                    vec![]
                }
            })
            .unwrap_or_default();
        evidence_effects.extend(self.on_new_vote(&vote_hash, now, rng));
        evidence_effects
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::iter::FromIterator;

    use crate::{
        components::consensus::{
            highway_core::{
                evidence::{Evidence, EvidenceError},
                highway::{Highway, SignedWireVote, Vertex, VertexError, VoteError, WireVote},
                state::{
                    tests::{
                        TestContext, TestSecret, ALICE, ALICE_SEC, BOB, BOB_SEC, CAROL, CAROL_SEC,
                        WEIGHTS,
                    },
                    Panorama, State,
                },
                validators::Validators,
            },
            traits::ValidatorSecret,
        },
        testing::TestRng,
        types::Timestamp,
    };

    fn test_validators() -> Validators<u32> {
        let vid_weights: Vec<(u32, u64)> =
            vec![(ALICE_SEC, ALICE), (BOB_SEC, BOB), (CAROL_SEC, CAROL)]
                .into_iter()
                .map(|(sk, vid)| {
                    assert_eq!(sk.0, vid.0);
                    (sk.0, WEIGHTS[vid.0 as usize].0)
                })
                .collect();
        Validators::from_iter(vid_weights)
    }

    #[test]
    fn invalid_signature_error() {
        let mut rng = TestRng::new();
        let now: Timestamp = 500.into();

        let state: State<TestContext> = State::new_test(WEIGHTS, 0);
        let mut highway = Highway {
            instance_id: 1u64,
            validators: test_validators(),
            state,
            active_validator: None,
        };
        let wvote = WireVote {
            panorama: Panorama::new(WEIGHTS.len()),
            creator: CAROL,
            instance_id: highway.instance_id,
            value: Some(0),
            seq_number: 0,
            timestamp: Timestamp::zero(),
            round_exp: 4,
            endorsed: vec![],
        };
        let invalid_signature = 1u64;
        let invalid_signature_vote = SignedWireVote {
            wire_vote: wvote.clone(),
            signature: invalid_signature,
        };
        let invalid_vertex = Vertex::Vote(invalid_signature_vote);
        let err = VertexError::Vote(VoteError::Signature);
        let expected = (invalid_vertex.clone(), err);
        assert_eq!(Err(expected), highway.pre_validate_vertex(invalid_vertex));

        // TODO: Also test the `missing_dependency` and `validate_vertex` steps.

        let valid_signature = CAROL_SEC.sign(&wvote.hash(), &mut rng);
        let correct_signature_vote = SignedWireVote {
            wire_vote: wvote,
            signature: valid_signature,
        };
        let valid_vertex = Vertex::Vote(correct_signature_vote);
        let pvv = highway.pre_validate_vertex(valid_vertex).unwrap();
        assert_eq!(None, highway.missing_dependency(&pvv));
        let vv = highway.validate_vertex(pvv).unwrap();
        assert!(highway.add_valid_vertex(vv, &mut rng, now).is_empty());
    }

    #[test]
    fn invalid_evidence() {
        let mut rng = TestRng::new();

        let state: State<TestContext> = State::new_test(WEIGHTS, 0);
        let highway = Highway {
            instance_id: 1u64,
            validators: test_validators(),
            state,
            active_validator: None,
        };

        let mut validate = |wvote0: &WireVote<TestContext>,
                            signer0: &TestSecret,
                            wvote1: &WireVote<TestContext>,
                            signer1: &TestSecret| {
            let swvote0 = SignedWireVote::new(wvote0.clone(), signer0, &mut rng);
            let swvote1 = SignedWireVote::new(wvote1.clone(), signer1, &mut rng);
            let evidence = Evidence::Equivocation(swvote0, swvote1);
            let vertex = Vertex::Evidence(evidence);
            highway
                .pre_validate_vertex(vertex.clone())
                .map_err(|(v, err)| {
                    assert_eq!(v, vertex);
                    err
                })
        };

        // Two votes with different values and the same sequence number. Carol equivocated!
        let mut wvote0 = WireVote {
            panorama: Panorama::new(WEIGHTS.len()),
            creator: CAROL,
            instance_id: highway.instance_id,
            value: Some(0),
            seq_number: 0,
            timestamp: Timestamp::zero(),
            round_exp: 4,
            endorsed: vec![],
        };
        let wvote1 = WireVote {
            panorama: Panorama::new(WEIGHTS.len()),
            creator: CAROL,
            instance_id: highway.instance_id,
            value: Some(1),
            seq_number: 0,
            timestamp: Timestamp::zero(),
            round_exp: 4,
            endorsed: vec![],
        };

        assert!(validate(&wvote0, &CAROL_SEC, &wvote1, &CAROL_SEC,).is_ok());

        // It's only an equivocation if the two votes are different.
        assert_eq!(
            Err(VertexError::Evidence(EvidenceError::EquivocationSameVote)),
            validate(&wvote0, &CAROL_SEC, &wvote0, &CAROL_SEC)
        );

        // Both votes have Carol as their creator; Bob's signature would be invalid.
        assert_eq!(
            Err(VertexError::Evidence(EvidenceError::Signature)),
            validate(&wvote0, &CAROL_SEC, &wvote1, &BOB_SEC)
        );
        assert_eq!(
            Err(VertexError::Evidence(EvidenceError::Signature)),
            validate(&wvote0, &BOB_SEC, &wvote1, &CAROL_SEC)
        );

        // If the first vote was actually Bob's and the second Carol's, nobody equivocated.
        wvote0.creator = BOB;
        assert_eq!(
            Err(VertexError::Evidence(
                EvidenceError::EquivocationDifferentCreators
            )),
            validate(&wvote0, &BOB_SEC, &wvote1, &CAROL_SEC)
        );
        wvote0.creator = CAROL;

        // If the votes have different sequence numbers they might belong to the same fork.
        wvote0.seq_number = 1;
        assert_eq!(
            Err(VertexError::Evidence(
                EvidenceError::EquivocationDifferentSeqNumbers
            )),
            validate(&wvote0, &CAROL_SEC, &wvote1, &CAROL_SEC)
        );
        wvote0.seq_number = 0;

        // If the votes are from a different network or era we don't accept the evidence.
        wvote0.instance_id = 2;
        assert_eq!(
            Err(VertexError::Evidence(EvidenceError::EquivocationInstanceId)),
            validate(&wvote0, &CAROL_SEC, &wvote1, &CAROL_SEC)
        );
    }
}
