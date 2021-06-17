use std::{
    fmt::{self, Debug},
    fs::{self, File},
    io::{self, Read, Write},
    iter,
    path::{Path, PathBuf},
};

use datasize::DataSize;
use tracing::{error, info, trace, warn};

use super::{
    endorsement::{Endorsement, SignedEndorsement},
    evidence::Evidence,
    highway::{Ping, ValidVertex, Vertex, WireUnit},
    state::{self, Panorama, State, Unit, Weight},
    validators::ValidatorIndex,
};

use crate::{
    components::consensus::{
        consensus_protocol::BlockContext,
        highway_core::{highway::SignedWireUnit, state::Fault},
        traits::{Context, ValidatorSecret},
    },
    types::{TimeDiff, Timestamp},
};

/// An action taken by a validator.
#[derive(Clone, Eq, PartialEq, Debug)]
pub(crate) enum Effect<C: Context> {
    /// Newly vertex that should be gossiped to peers and added to the protocol state.
    NewVertex(ValidVertex<C>),
    /// `handle_timer` needs to be called at the specified time.
    ScheduleTimer(Timestamp),
    /// `propose` needs to be called with a value for a new block with the specified block context
    /// and parent value.
    RequestNewBlock(BlockContext<C>),
    /// This validator is faulty.
    ///
    /// When this is returned, the validator automatically deactivates.
    WeAreFaulty(Fault<C>),
}

/// A validator that actively participates in consensus by creating new vertices.
///
/// It implements the Highway schedule. The protocol proceeds in rounds, and in each round one
/// validator is the _leader_.
/// * In the beginning of the round, the leader sends a _proposal_ unit, containing a consensus
///   value (i.e. a block).
/// * Upon receiving the proposal, all the other validators send a _confirmation_ unit, citing only
///   the proposal, their own previous message, and resulting transitive justifications.
/// * At a fixed point in time later in the round, everyone unconditionally sends a _witness_ unit,
///   citing every unit they have received so far.
///
/// If the rounds are long enough (i.e. message delivery is fast enough) and there are enough
/// honest validators, there will be a lot of confirmations for the proposal, and enough witness
/// units citing all those confirmations, to create a summit and finalize the proposal.
#[derive(DataSize)]
pub(crate) struct ActiveValidator<C>
where
    C: Context,
{
    /// Our own validator index.
    vidx: ValidatorIndex,
    /// The validator's secret signing key.
    secret: C::ValidatorSecret,
    /// The next round exponent: Our next round will be `1 << next_round_exp` milliseconds long.
    next_round_exp: u8,
    /// The latest timer we scheduled.
    next_timer: Timestamp,
    /// Panorama and context for a block we are about to propose when we get a consensus value.
    next_proposal: Option<(BlockContext<C>, Panorama<C>)>,
    /// The path to the file storing the hash of our latest known unit (if any).
    unit_file: Option<PathBuf>,
    /// The last known unit created by us.
    own_last_unit: Option<(SignedWireUnit<C>, Panorama<C>)>,
    /// The target fault tolerance threshold. The validator pauses (i.e. doesn't create new units)
    /// if not enough validators are online to finalize values at this FTT.
    target_ftt: Weight,
    /// If this flag is set we don't create new units and just send pings instead.
    paused: bool,
}

impl<C: Context> Debug for ActiveValidator<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActiveValidator")
            .field("vidx", &self.vidx)
            .field("next_round_exp", &self.next_round_exp)
            .field("next_timer", &self.next_timer)
            .field("paused", &self.paused)
            .finish()
    }
}

impl<C: Context> ActiveValidator<C> {
    /// Creates a new `ActiveValidator` and the timer effect for the first call.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        vidx: ValidatorIndex,
        secret: C::ValidatorSecret,
        current_time: Timestamp,
        start_time: Timestamp,
        state: &State<C>,
        unit_file: Option<PathBuf>,
        target_ftt: Weight,
        instance_id: C::InstanceId,
    ) -> (Self, Vec<Effect<C>>) {
        let own_last_unit = unit_file
            .as_ref()
            .map(read_last_unit)
            .transpose()
            .map_err(|err| match err.kind() {
                io::ErrorKind::NotFound => (),
                _ => panic!("got an error reading unit file {:?}: {:?}", unit_file, err),
            })
            .ok()
            .flatten();
        let mut av = ActiveValidator {
            vidx,
            secret,
            next_round_exp: state.params().init_round_exp(),
            next_timer: state.params().start_timestamp(),
            next_proposal: None,
            unit_file,
            own_last_unit,
            target_ftt,
            paused: false,
        };
        let mut effects = av.schedule_timer(start_time, state);
        effects.push(av.send_ping(current_time, instance_id));
        (av, effects)
    }

    /// Returns whether validator's protocol state is fully synchronized and it's safe to start
    /// creating units.
    ///
    /// If validator restarted within an era, it most likely had created units before that event. It
    /// cannot start creating new units until its state is fully synchronized, otherwise it will
    /// most likely equivocate.
    fn can_vote(&self, state: &State<C>) -> bool {
        self.own_last_unit
            .as_ref()
            .map_or(true, |(swunit, _)| state.has_unit(&swunit.hash()))
    }

    /// Returns whether validator's protocol state is synchronized up until the panorama of its own
    /// last unit.
    pub(crate) fn is_own_last_unit_panorama_sync(&self, state: &State<C>) -> bool {
        self.own_last_unit.as_ref().map_or(true, |(_, panorama)| {
            panorama
                .iter_correct_hashes()
                .all(|hash| state.has_unit(hash))
        })
    }

    pub(crate) fn take_own_last_unit(&mut self) -> Option<(SignedWireUnit<C>, Panorama<C>)> {
        self.own_last_unit.take()
    }

    /// Sets the next round exponent to the new value.
    pub(crate) fn set_round_exp(&mut self, new_round_exp: u8) {
        self.next_round_exp = new_round_exp;
    }

    /// Sets the pause status: While paused we don't create any new units, just pings.
    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused
    }

    /// Returns actions a validator needs to take at the specified `timestamp`, with the given
    /// protocol `state`.
    pub(crate) fn handle_timer(
        &mut self,
        timestamp: Timestamp,
        state: &State<C>,
        instance_id: C::InstanceId,
    ) -> Vec<Effect<C>> {
        if self.is_faulty(state) {
            warn!("Creator knows it's faulty. Won't create a message.");
            return vec![];
        }
        let mut effects = self.schedule_timer(timestamp, state);
        if self.earliest_unit_time(state) > timestamp {
            warn!(%timestamp, "skipping outdated timer event");
            return effects;
        }
        let r_exp = self.round_exp(state, timestamp);
        let r_id = state::round_id(timestamp, r_exp);
        let r_len = state::round_len(r_exp);
        // Only create new units if enough validators are online.
        if !self.paused && self.enough_validators_online(state, timestamp) {
            if timestamp == r_id && state.leader(r_id) == self.vidx {
                effects.extend(self.request_new_block(state, instance_id, timestamp));
                return effects;
            } else if timestamp == r_id + self.witness_offset(r_len) {
                let panorama = self.panorama_at(state, timestamp);
                if let Some(witness_vv) =
                    self.new_unit(panorama, timestamp, None, state, instance_id)
                {
                    if self
                        .latest_unit(state)
                        .map_or(true, |latest_unit| latest_unit.round_id() != r_id)
                    {
                        info!(round_id = %r_id, "sending witness in round with no proposal");
                    }
                    effects.push(Effect::NewVertex(witness_vv));
                    return effects;
                }
            }
        }
        // We are not creating a new unit. Send a ping if necessary, to show that we're online.
        if !state.has_ping(self.vidx, timestamp) {
            warn!(%timestamp, "too many validators offline, sending ping");
            effects.push(self.send_ping(timestamp, instance_id));
        }
        effects
    }

    /// Creates a Ping vertex.
    pub(crate) fn send_ping(&self, timestamp: Timestamp, instance_id: C::InstanceId) -> Effect<C> {
        let ping = Ping::new(self.vidx, timestamp, instance_id, &self.secret);
        Effect::NewVertex(ValidVertex(Vertex::Ping(ping)))
    }

    /// Returns whether enough validators are online to finalize values with the target fault
    /// tolerance threshold, always counting this validator as online.
    fn enough_validators_online(&self, state: &State<C>, now: Timestamp) -> bool {
        // We divide before adding, because  total_weight + target_fft  could overflow u64.
        let target_quorum = state.total_weight() / 2 + self.target_ftt / 2;
        let online_weight: Weight = state
            .weights()
            .enumerate()
            .filter(|(vidx, _)| {
                self.vidx == *vidx || (!state.is_faulty(*vidx) && state.is_online(*vidx, now))
            })
            .map(|(_, w)| *w)
            .sum();
        online_weight > target_quorum
    }

    /// Returns actions a validator needs to take upon receiving a new unit.
    pub(crate) fn on_new_unit(
        &mut self,
        uhash: &C::Hash,
        now: Timestamp,
        state: &State<C>,
        instance_id: C::InstanceId,
    ) -> Vec<Effect<C>> {
        if let Some(fault) = state.maybe_fault(self.vidx) {
            return vec![Effect::WeAreFaulty(fault.clone())];
        }
        let mut effects = vec![];
        if self.should_send_confirmation(uhash, now, state) {
            let panorama = state.confirmation_panorama(self.vidx, uhash);
            if panorama.has_correct() {
                if let Some(confirmation_vv) =
                    self.new_unit(panorama, now, None, state, instance_id)
                {
                    effects.push(Effect::NewVertex(confirmation_vv));
                }
            }
        };
        if self.should_endorse(uhash, state) {
            let endorsement = self.endorse(uhash);
            effects.extend(vec![Effect::NewVertex(ValidVertex(endorsement))]);
        }
        effects
    }

    /// Returns actions validator needs to take upon receiving a new evidence.
    /// Endorses all latest units by honest validators that do not mark new perpetrator as faulty
    /// and cite some new message by that validator.
    pub(crate) fn on_new_evidence(
        &mut self,
        evidence: &Evidence<C>,
        state: &State<C>,
    ) -> Vec<Effect<C>> {
        let vidx = evidence.perpetrator();
        state
            .iter_correct_hashes()
            .filter(|&v| {
                let unit = state.unit(v);
                unit.new_hash_obs(state, vidx)
            })
            .map(|v| self.endorse(v))
            .map(|endorsement| Effect::NewVertex(ValidVertex(endorsement)))
            .collect()
    }

    /// Returns an effect to request a consensus value for a block to propose.
    ///
    /// If we are already waiting for a consensus value, `None` is returned instead.
    /// If the new value would come after a terminal block, the proposal is made immediately, and
    /// without a value.
    fn request_new_block(
        &mut self,
        state: &State<C>,
        instance_id: C::InstanceId,
        timestamp: Timestamp,
    ) -> Option<Effect<C>> {
        if let Some((prop_context, _)) = self.next_proposal.take() {
            warn!(?prop_context, "no proposal received; requesting new one");
        }
        let panorama = self.panorama_at(state, timestamp);
        let maybe_parent_hash = state.fork_choice(&panorama);
        // If the parent is a terminal block, just create a unit without a new block.
        if maybe_parent_hash.map_or(false, |hash| state.is_terminal_block(hash)) {
            return self
                .new_unit(panorama, timestamp, None, state, instance_id)
                .map(Effect::NewVertex);
        }
        // Otherwise we need to request a new consensus value to propose.
        let ancestor_values = match maybe_parent_hash {
            None => vec![],
            Some(parent_hash) => iter::once(parent_hash)
                .chain(state.ancestor_hashes(parent_hash))
                .map(|bhash| state.block(bhash).value.clone())
                .collect(),
        };
        let block_context = BlockContext::new(timestamp, ancestor_values);
        self.next_proposal = Some((block_context.clone(), panorama));
        Some(Effect::RequestNewBlock(block_context))
    }

    /// Proposes a new block with the given consensus value.
    pub(crate) fn propose(
        &mut self,
        value: C::ConsensusValue,
        block_context: BlockContext<C>,
        state: &State<C>,
        instance_id: C::InstanceId,
    ) -> Vec<Effect<C>> {
        let timestamp = block_context.timestamp();
        let panorama = if let Some((expected_context, panorama)) = self.next_proposal.take() {
            if expected_context != block_context {
                warn!(?expected_context, ?block_context, "unexpected proposal");
                return vec![];
            }
            panorama
        } else {
            warn!("unexpected proposal value");
            return vec![];
        };
        if self.earliest_unit_time(state) > timestamp {
            warn!(?block_context, "skipping outdated proposal");
            return vec![];
        }
        if self.is_faulty(state) {
            warn!("Creator knows it's faulty. Won't create a message.");
            return vec![];
        }
        self.new_unit(panorama, timestamp, Some(value), state, instance_id)
            .map(Effect::NewVertex)
            .into_iter()
            .collect()
    }

    /// Returns whether the incoming message is a proposal that we need to send a confirmation for.
    fn should_send_confirmation(
        &self,
        vhash: &C::Hash,
        timestamp: Timestamp,
        state: &State<C>,
    ) -> bool {
        let unit = state.unit(vhash);
        // If it's not a proposal, the sender is faulty, or we are, don't send a confirmation.
        if unit.creator == self.vidx || self.is_faulty(state) || !state.is_correct_proposal(unit) {
            return false;
        }
        let r_id = state::round_id(timestamp, self.round_exp(state, timestamp));
        if unit.timestamp != r_id {
            trace!(
                %unit.timestamp, %r_id,
                "not confirming proposal: wrong round",
            );
            return false;
        }
        if unit.timestamp > timestamp {
            error!(
                %unit.timestamp, %timestamp,
                "added a unit with a future timestamp, should never happen"
            );
            return false;
        }
        if let Some(unit) = self.latest_unit(state) {
            if unit.panorama.sees_correct(state, vhash) {
                error!(%vhash, "called on_new_unit with already confirmed proposal");
                return false; // We already sent a confirmation.
            }
        }
        let earliest_unit_time = self.earliest_unit_time(state);
        if timestamp < earliest_unit_time {
            warn!(
                %earliest_unit_time, %timestamp,
                "earliest_unit_time is greater than current time stamp"
            );
            return false;
        }
        true
    }

    /// Returns a new unit with the given data, and the correct sequence number.
    ///
    /// Returns `None` if it's not possible to create a valid unit with the given panorama.
    fn new_unit(
        &mut self,
        panorama: Panorama<C>,
        timestamp: Timestamp,
        value: Option<C::ConsensusValue>,
        state: &State<C>,
        instance_id: C::InstanceId,
    ) -> Option<ValidVertex<C>> {
        if value.is_none() && !panorama.has_correct() {
            return None; // Wait for the first proposal before creating a unit without a value.
        }
        if !self.can_vote(state) {
            info!(?self.own_last_unit, "not voting - last own unit unknown");
            return None;
        }
        if let Some((prop_context, _)) = self.next_proposal.take() {
            warn!(?prop_context, "canceling proposal due to unit");
        }
        for hash in panorama.iter_correct_hashes() {
            if timestamp < state.unit(hash).timestamp {
                error!(
                    %timestamp, justification_timestamp = %state.unit(hash).timestamp,
                    "canceling unit creation because of outdated timestamp"
                );
                return None;
            }
        }
        if panorama[self.vidx] != state.panorama()[self.vidx] {
            error!(
                ?panorama,
                "panorama for new unit would be equivocation; canceling unit creation"
            );
            return None;
        }
        let previous = panorama[self.vidx].correct().cloned();
        let seq_number = panorama.next_seq_num(state, self.vidx);
        let endorsed = state.seen_endorsed(&panorama);
        let panorama_hash = panorama.hash();
        let seq_num_panorama = panorama.to_seq_num_panorama(&state);
        let hwunit = WireUnit {
            seq_num_panorama,
            panorama_hash,
            previous,
            creator: self.vidx,
            instance_id,
            value,
            seq_number,
            timestamp,
            round_exp: self.round_exp(state, timestamp),
            endorsed,
        }
        .into_hashed();
        let swunit = SignedWireUnit::new(hwunit, &self.secret);
        write_last_unit(&self.unit_file, &swunit, &panorama).unwrap_or_else(|err| {
            panic!(
                "should successfully write unit's hash to {:?}, got {:?}",
                self.unit_file, err
            )
        });
        Some(ValidVertex(Vertex::UnitWithPanorama(swunit, panorama)))
    }

    /// Returns a `ScheduleTimer` effect for the next time we need to be called.
    ///
    /// If the time is before the current round's witness unit, schedule the witness unit.
    /// Otherwise, if we are the next round's leader, schedule the proposal unit.
    /// Otherwise schedule the next round's witness unit.
    fn schedule_timer(&mut self, timestamp: Timestamp, state: &State<C>) -> Vec<Effect<C>> {
        if self.next_timer > timestamp {
            return Vec::new(); // We already scheduled the next call; nothing to do.
        }
        let r_exp = self.round_exp(state, timestamp);
        let r_id = state::round_id(timestamp, r_exp);
        let r_len = state::round_len(r_exp);
        self.next_timer = if timestamp < r_id + self.witness_offset(r_len) {
            r_id + self.witness_offset(r_len)
        } else {
            let next_r_id = r_id + r_len;
            if state.leader(next_r_id) == self.vidx {
                next_r_id
            } else {
                let next_r_exp = self.round_exp(state, next_r_id);
                next_r_id + self.witness_offset(state::round_len(next_r_exp))
            }
        };
        vec![Effect::ScheduleTimer(self.next_timer)]
    }

    /// Returns the earliest timestamp where we can cast our next unit: It can't be earlier than
    /// our previous unit, and it can't be the third unit in a single round.
    fn earliest_unit_time(&self, state: &State<C>) -> Timestamp {
        self.latest_unit(state)
            .map_or(state.params().start_timestamp(), |unit| {
                unit.previous().map_or(unit.timestamp, |vh2| {
                    let unit2 = state.unit(vh2);
                    unit.timestamp.max(unit2.round_id() + unit2.round_len())
                })
            })
    }

    /// Returns the most recent unit by this validator.
    pub(crate) fn latest_unit<'a>(&self, state: &'a State<C>) -> Option<&'a Unit<C>> {
        state
            .panorama()
            .get(self.vidx)
            .correct()
            .map(|vh| state.unit(vh))
    }

    /// Checks if validator knows it's faulty.
    fn is_faulty(&self, state: &State<C>) -> bool {
        state.panorama().get(self.vidx).is_faulty()
    }

    /// Returns the duration after the beginning of a round when the witness units are sent.
    fn witness_offset(&self, round_len: TimeDiff) -> TimeDiff {
        round_len * 2 / 3
    }

    /// The round exponent of the round containing `timestamp`.
    ///
    /// This returns `self.next_round_exp`, if that is a valid round exponent for a unit cast at
    /// `timestamp`. Otherwise it returns the round exponent of our latest unit.
    fn round_exp(&self, state: &State<C>, timestamp: Timestamp) -> u8 {
        self.latest_unit(state).map_or(self.next_round_exp, |unit| {
            let max_re = self.next_round_exp.max(unit.round_exp);
            if unit.timestamp < state::round_id(timestamp, max_re) {
                self.next_round_exp
            } else {
                unit.round_exp
            }
        })
    }

    /// Returns whether we should endorse the `vhash`.
    ///
    /// We should endorse unit from honest validator that cites _an_ equivocator
    /// as honest and it cites some new message by that validator.
    fn should_endorse(&self, vhash: &C::Hash, state: &State<C>) -> bool {
        let unit = state.unit(vhash);
        !state.is_faulty(unit.creator)
            && unit
                .panorama
                .enumerate()
                .any(|(vidx, _)| state.is_faulty(vidx) && unit.new_hash_obs(state, vidx))
    }

    /// Creates endorsement of the `vhash`.
    fn endorse(&self, vhash: &C::Hash) -> Vertex<C> {
        let endorsement = Endorsement::new(*vhash, self.vidx);
        let signature = self.secret.sign(&endorsement.hash());
        Vertex::Endorsements(SignedEndorsement::new(endorsement, signature).into())
    }

    /// Returns a panorama that is valid to use in our own unit at the given timestamp.
    fn panorama_at(&self, state: &State<C>, timestamp: Timestamp) -> Panorama<C> {
        // Take the panorama of all units at or before the given timestamp, because it's invalid to
        // cite units newer than that. This is only relevant if we added units to the state whose
        // timestamp is newer than the one of the unit we are creating, but it can happen due to
        // delayed timer events.
        let past_panorama = state.panorama().cutoff(state, timestamp);
        state.valid_panorama(self.vidx, past_panorama)
    }

    /// Returns whether the unit was created by us.
    pub(crate) fn is_our_unit(&self, wunit: &WireUnit<C>) -> bool {
        self.vidx == wunit.creator
    }

    /// Returns whether the incoming vertex was signed by our key even though we don't have it yet.
    /// This can only happen if another node is running with the same signing key.
    pub(crate) fn is_doppelganger_vertex(&self, vertex: &Vertex<C>, state: &State<C>) -> bool {
        if !self.can_vote(state) {
            return false;
        }
        match vertex {
            Vertex::Unit(swunit) | Vertex::UnitWithPanorama(swunit, _) => {
                // If we already have the unit in our local state,
                // we must have had created it ourselves earlier and it is now gossiped back to us.
                self.is_our_unit(swunit.wire_unit()) && !state.has_unit(&swunit.hash())
            }
            Vertex::Endorsements(endorsements) => {
                if state::TODO_ENDORSEMENT_EVIDENCE_DISABLED {
                    return false;
                }
                // Check whether the list of endorsements includes one created by a doppelganger.
                // An endorsement created by a doppelganger cannot be found in the local protocol
                // state (since we haven't created it ourselves).
                let is_ours = |(vidx, _): &(ValidatorIndex, _)| vidx == &self.vidx;
                endorsements.endorsers.iter().any(is_ours)
                    && !state.has_endorsement(endorsements.unit(), self.vidx)
            }
            Vertex::Ping(ping) => {
                ping.creator() == self.vidx && !state.has_ping(self.vidx, ping.timestamp())
            }
            Vertex::Evidence(_) => false,
        }
    }

    pub(crate) fn next_round_length(&self) -> TimeDiff {
        state::round_len(self.next_round_exp)
    }
}

pub(crate) fn read_last_unit<C, P>(path: P) -> io::Result<(SignedWireUnit<C>, Panorama<C>)>
where
    C: Context,
    P: AsRef<Path>,
{
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn write_last_unit<C: Context>(
    unit_file: &Option<PathBuf>,
    swunit: &SignedWireUnit<C>,
    panorama: &Panorama<C>,
) -> io::Result<()> {
    // If there is no unit_file set, do not write to it
    let unit_file = if let Some(file) = unit_file.as_ref() {
        file
    } else {
        return Ok(());
    };

    // Create the file (and its parents) as necessary
    if let Some(parent_directory) = unit_file.parent() {
        fs::create_dir_all(parent_directory)?;
    }
    let mut file = File::create(unit_file)?;

    // Finally, write the data to file we created
    let bytes = serde_json::to_vec(&(swunit, panorama))?;

    file.write_all(&bytes)
}

#[cfg(test)]
#[allow(clippy::integer_arithmetic)] // Overflows in tests panic anyway.
mod tests {
    use std::{collections::BTreeSet, fmt::Debug};
    use tempfile::tempdir;

    use crate::components::consensus::highway_core::{
        highway::ObservationSeqNum, highway_testing::TEST_INSTANCE_ID, validators::ValidatorMap,
    };

    use super::{
        super::{
            finality_detector::FinalityDetector,
            state::{tests::*, State, Weight},
        },
        *,
    };

    type Eff = Effect<TestContext>;

    impl Eff {
        fn unwrap_unit(self) -> (SignedWireUnit<TestContext>, Panorama<TestContext>) {
            match self {
                Eff::NewVertex(ValidVertex(Vertex::UnitWithPanorama(swunit, panorama))) => {
                    (swunit, panorama)
                }
                _ => panic!("Unexpected effect: {:?}", self),
            }
        }

        fn unwrap_timer(self) -> Timestamp {
            if let Eff::ScheduleTimer(timestamp) = self {
                timestamp
            } else {
                panic!("expected `ScheduleTimer`, got: {:?}", self)
            }
        }
    }

    struct TestState {
        state: State<TestContext>,
        instance_id: u64,
        fd: FinalityDetector<TestContext>,
        active_validators: ValidatorMap<ActiveValidator<TestContext>>,
        timers: BTreeSet<(Timestamp, ValidatorIndex)>,
    }

    impl TestState {
        fn new(
            state: State<TestContext>,
            start_time: Timestamp,
            instance_id: u64,
            fd: FinalityDetector<TestContext>,
            validators: Vec<ValidatorIndex>,
        ) -> Self {
            let mut timers = BTreeSet::new();
            let current_round_id = state::round_id(start_time, state.params().init_round_exp());
            let earliest_round_start = if start_time == current_round_id {
                start_time
            } else {
                current_round_id + state::round_len(state.params().init_round_exp())
            };
            let target_ftt = state.total_weight() / 3;
            let active_validators = validators
                .into_iter()
                .map(|vidx| {
                    let secret = TestSecret(vidx.0);
                    let (av, effects) = ActiveValidator::new(
                        vidx,
                        secret,
                        start_time,
                        start_time,
                        &state,
                        None,
                        target_ftt,
                        TEST_INSTANCE_ID,
                    );

                    let timestamp = match &*effects {
                        [
                            Effect::ScheduleTimer(timer),
                            Effect::NewVertex(ValidVertex(Vertex::Ping(_)))
                        ] => { *timer }
                        other => panic!("expected timer and ping effects, got={:?}", other),
                    };

                    if state.leader(earliest_round_start) == vidx {
                        assert_eq!(
                            timestamp, earliest_round_start,
                            "Invalid initial timer scheduled for {:?}.",
                            vidx,
                        )
                    } else {
                        let witness_offset =
                            av.witness_offset(state::round_len(state.params().init_round_exp()));
                        let witness_timestamp = earliest_round_start + witness_offset;
                        assert_eq!(
                            timestamp, witness_timestamp,
                            "Invalid initial timer scheduled for {:?}.",
                            vidx,
                        )
                    }
                    timers.insert((timestamp, vidx));
                    av
                })
                .collect();

            TestState {
                state,
                instance_id,
                fd,
                active_validators,
                timers,
            }
        }

        /// Force the validator to handle timer that may not have been scheduled by it.
        /// Useful for testing.
        /// Returns effects created when handling the timer.
        fn handle_timer(
            &mut self,
            vidx: ValidatorIndex,
            timestamp: Timestamp,
        ) -> Vec<Effect<TestContext>> {
            // Remove the timer from the queue if it has been scheduled.
            let _ = self.timers.remove(&(timestamp, vidx));
            let validator = &mut self.active_validators[vidx];
            let effects = validator.handle_timer(timestamp, &self.state, self.instance_id);
            self.schedule_timer(vidx, &effects);
            self.add_new_unit(&effects);
            effects
        }

        /// Propose new consensus value as validator `vidx`.
        /// Returns effects created when proposing and newly proposed wire unit.
        fn propose(
            &mut self,
            vidx: ValidatorIndex,
            cv: <TestContext as Context>::ConsensusValue,
            block_context: BlockContext<TestContext>,
        ) -> (Vec<Effect<TestContext>>, SignedWireUnit<TestContext>) {
            let validator = &mut self.active_validators[vidx];
            let proposal_timestamp = block_context.timestamp();
            let effects = validator.propose(cv, block_context, &self.state, self.instance_id);

            // Add the new unit to the state.
            let (proposal_wunit, panorama) = unwrap_single(&effects).unwrap_unit();
            let prop_hash = proposal_wunit.hash();
            self.state
                .add_unit(proposal_wunit.clone(), panorama)
                .unwrap();
            let effects = validator.on_new_unit(
                &prop_hash,
                proposal_timestamp + 1.into(),
                &self.state,
                self.instance_id,
            );
            self.schedule_timer(vidx, &effects);
            (effects, proposal_wunit)
        }

        /// Handle new unit by validator `vidx`.
        /// Since all validators use the same state, that unit should be added already. Panics if
        /// not. Returns effect created when handling new unit.
        fn handle_new_unit(
            &mut self,
            vidx: ValidatorIndex,
            uhash: &<TestContext as Context>::Hash,
        ) -> Vec<Effect<TestContext>> {
            let validator = &mut self.active_validators[vidx];
            let delivery_timestamp = self.state.unit(uhash).timestamp + 1.into();
            let effects =
                validator.on_new_unit(uhash, delivery_timestamp, &self.state, self.instance_id);
            self.schedule_timer(vidx, &effects);
            self.add_new_unit(&effects);
            effects
        }

        /// Schedules new timers, if any was returned as an effect.
        fn schedule_timer(&mut self, vidx: ValidatorIndex, effects: &[Effect<TestContext>]) {
            let new_timestamps: Vec<Timestamp> = effects
                .iter()
                .filter_map(|eff| {
                    if let Effect::ScheduleTimer(timestamp) = eff {
                        Some(*timestamp)
                    } else {
                        None
                    }
                })
                .collect();
            match *new_timestamps {
                [] => (),
                [timestamp] => {
                    let _ = self.timers.insert((timestamp, vidx));
                }
                _ => panic!(
                    "Expected at most one timer to be scheduled: {:?}",
                    new_timestamps
                ),
            }
        }

        /// Adds new unit, if any, to the state.
        fn add_new_unit(&mut self, effects: &[Effect<TestContext>]) {
            let new_units: Vec<_> = effects
                .iter()
                .filter_map(|eff| match eff {
                    Effect::NewVertex(ValidVertex(Vertex::UnitWithPanorama(swunit, panorama))) => {
                        Some((swunit, panorama))
                    }
                    _ => None,
                })
                .collect();
            match *new_units {
                [] => (),
                [(unit, panorama)] => {
                    let _ = self.state.add_unit(unit.clone(), panorama.clone()).unwrap();
                }
                _ => panic!("Expected at most one unit to be added: {:?}", new_units),
            }
        }

        /// Returns hash of the newly finalized unit.
        fn next_finalized(&mut self) -> Option<&<TestContext as Context>::Hash> {
            self.fd.next_finalized(&self.state)
        }
    }

    fn unwrap_single<T: Debug + Clone>(vec: &[T]) -> T {
        let mut iter = vec.iter();
        match (iter.next(), iter.next()) {
            (None, _) => panic!("Unexpected empty vec"),
            (Some(t), None) => t.clone(),
            (Some(t0), Some(t1)) => panic!("Expected only one element: {:?}, {:?}", t0, t1),
        }
    }

    #[test]
    #[allow(clippy::unreadable_literal)] // 0xC0FFEE is more readable than 0x00C0_FFEE.
    fn active_validator() {
        let mut test = TestState::new(
            State::new_test(&[Weight(3), Weight(4)], 0),
            410.into(),
            1u64,
            FinalityDetector::new(Weight(2)),
            vec![ALICE, BOB],
        );

        assert!(test.handle_timer(ALICE, 415.into()).is_empty()); // Too early: No new effects.

        // We start at time 410, with round length 16, so the first leader tick is
        // 416, and the first witness tick 426.
        // Alice wants to propose a block, and also make her witness unit at 426.
        let bctx = match &*test.handle_timer(ALICE, 416.into()) {
            [Eff::ScheduleTimer(timestamp), Eff::RequestNewBlock(bctx)]
                if *timestamp == 426.into() =>
            {
                bctx.clone()
            }
            effects => panic!("unexpected effects {:?}", effects),
        };
        assert_eq!(
            Timestamp::from(416),
            bctx.timestamp(),
            "Proposal should be scheduled for the expected timestamp."
        );

        // She has a pending deploy from Colin who wants to pay for a hot beverage.
        let (effects, new_unit) = test.propose(ALICE, 0xC0FFEE, bctx);
        assert!(
            effects.is_empty(),
            "No effects by creator after proposing a unit."
        );

        // Bob creates a confirmation unit for Alice's proposal.
        let effects = test.handle_new_unit(BOB, &new_unit.hash());
        // Validate that `effects` contain only one new unit – that is Bob's confirmation of Alice's
        // vote.
        let _ = unwrap_single(&effects).unwrap_unit();

        // Bob creates his witness message 2/3 through the round.
        let mut effects = test.handle_timer(BOB, 426.into()).into_iter();
        assert_eq!(Some(Eff::ScheduleTimer(432.into())), effects.next()); // Bob is the next leader.
        let _ = effects.next().unwrap().unwrap_unit();
        assert_eq!(None, effects.next());

        // Alice has not witnessed Bob's unit yet.
        assert_eq!(None, test.next_finalized());

        // Alice also sends her own witness message, completing the summit for her proposal.
        let mut effects = test.handle_timer(ALICE, 426.into()).into_iter();
        assert_eq!(Some(Eff::ScheduleTimer(442.into())), effects.next()); // Timer for witness unit.
        let _ = effects.next().unwrap().unwrap_unit();
        assert_eq!(None, effects.next());

        // Payment finalized! "One Pumpkin Spice Mochaccino for Corbyn!"
        assert_eq!(Some(&new_unit.hash()), test.next_finalized());
    }

    #[test]
    fn ping_on_startup() {
        let state = State::new_test(&[Weight(3)], 0);
        let (_alice, init_effects) = ActiveValidator::new(
            ALICE,
            TestSecret(ALICE.0),
            410.into(),
            410.into(),
            &state,
            None,
            Weight(2),
            TEST_INSTANCE_ID,
        );

        match &*init_effects {
            &[Effect::ScheduleTimer(_), Effect::NewVertex(ValidVertex(Vertex::Ping(_)))] => {}
            other => panic!(
                "expected two effects on startup: timer and ping. Got {:?}",
                other
            ),
        }
    }

    #[test]
    fn waits_until_synchronized() -> Result<(), AddUnitError<TestContext>> {
        let instance_id = TEST_INSTANCE_ID;
        let mut state = State::new_test(&[Weight(3)], 0);
        let a0 = {
            let a0 = add_unit!(state, ALICE, 0xB0; N)?;
            state.wire_unit(&a0, instance_id).unwrap()
        };
        let a1 = {
            let a1 = add_unit!(state, ALICE, None; a0.hash())?;
            state.wire_unit(&a1, instance_id).unwrap()
        };
        let a2 = {
            let a2 = add_unit!(state, ALICE, None; a1.hash())?;
            state.wire_unit(&a2, instance_id).unwrap()
        };
        let a0_panorama = state.unit(&a0.hash()).panorama.clone();
        let a1_panorama = state.unit(&a1.hash()).panorama.clone();
        let a2_panorama = state.unit(&a2.hash()).panorama.clone();
        // Clean state. We want Alice to synchronize first.
        state.retain_evidence_only();

        let unit_file = {
            let tmp_dir = tempdir().unwrap();
            let unit_hashes_folder = tmp_dir.path().to_path_buf();
            Some(unit_hashes_folder.join(format!("unit_hash_{:?}.dat", instance_id)))
        };

        // Store `a2` unit as the Alice's last unit.
        write_last_unit(&unit_file, &a2, &a2_panorama).expect("storing unit should succeed");
        // Alice's last unit is `a2` but `State` is empty. She must synchronize first.
        let (mut alice, alice_init_effects) = ActiveValidator::new(
            ALICE,
            TestSecret(ALICE.0),
            410.into(),
            410.into(),
            &state,
            unit_file,
            Weight(2),
            TEST_INSTANCE_ID,
        );

        let mut next_proposal_timer = match &*alice_init_effects {
            &[Effect::ScheduleTimer(timestamp), Effect::NewVertex(ValidVertex(Vertex::Ping(_)))]
                if timestamp == 416.into() =>
            {
                timestamp
            }
            other => panic!("unexpected effects {:?}", other),
        };

        // Alice has to synchronize up until `a2` (including) before she starts proposing.
        for (unit, panorama) in vec![
            (a0, a0_panorama),
            (a1, a1_panorama),
            (a2.clone(), a2_panorama),
        ] {
            next_proposal_timer =
                assert_no_proposal(&mut alice, &state, instance_id, next_proposal_timer);
            state.add_unit(unit, panorama)?;
        }

        // After synchronizing the protocol state up until `last_own_unit`, Alice can now propose a
        // new block.
        let bctx = match &*alice.handle_timer(next_proposal_timer, &state, instance_id) {
            [Eff::ScheduleTimer(_), Eff::RequestNewBlock(bctx)] => bctx.clone(),
            effects => panic!("unexpected effects {:?}", effects),
        };

        let (proposal_wunit, _panorama) =
            unwrap_single(&alice.propose(0xC0FFEE, bctx, &state, instance_id)).unwrap_unit();
        assert_eq!(
            proposal_wunit.wire_unit().seq_number,
            a2.wire_unit().seq_number + 1,
            "new unit should have correct seq_number"
        );
        assert_eq!(
            proposal_wunit.wire_unit().seq_num_panorama[ALICE],
            ObservationSeqNum::Correct(2),
            "new unit should cite the latest unit"
        );

        Ok(())
    }

    // Triggers new proposal by `validator` and verifies that it's empty – no block was proposed.
    // Captures the next witness timer and calls the `validator` with that to return the timer for
    // the next proposal.
    fn assert_no_proposal(
        validator: &mut ActiveValidator<TestContext>,
        state: &State<TestContext>,
        instance_id: u64,
        proposal_timer: Timestamp,
    ) -> Timestamp {
        let (witness_timestamp, bctx) =
            match &*validator.handle_timer(proposal_timer, &state, instance_id) {
                [Eff::ScheduleTimer(witness_timestamp), Eff::RequestNewBlock(bctx)] => {
                    (*witness_timestamp, bctx.clone())
                }
                effects => panic!("unexpected effects {:?}", effects),
            };

        let effects = validator.propose(0xC0FFEE, bctx, state, instance_id);
        assert!(
            effects.is_empty(),
            "should not propose blocks until its dependencies are synchronized: {:?}",
            effects
        );

        unwrap_single(&validator.handle_timer(witness_timestamp, &state, instance_id))
            .unwrap_timer()
    }
}
