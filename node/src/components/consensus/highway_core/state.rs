mod block;
mod panorama;
mod params;
mod tallies;
mod unit;
mod weight;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use params::Params;
use quanta::Clock;
pub(crate) use weight::Weight;

pub(crate) use panorama::{Observation, Panorama};
pub(super) use unit::Unit;

use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    iter,
};

use datasize::DataSize;
use itertools::Itertools;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use thiserror::Error;
use tracing::{error, info, trace, warn};

use crate::{
    components::consensus::{
        highway_core::{
            endorsement::{Endorsement, SignedEndorsement},
            evidence::Evidence,
            highway::{
                Dependency, Endorsements, HashedWireUnit, ObservationSeqNum, SignedWireUnit,
                WireUnit,
            },
            validators::{ValidatorIndex, ValidatorMap},
        },
        traits::Context,
    },
    types::{TimeDiff, Timestamp},
    utils::ds,
};
use block::Block;
use tallies::Tallies;

// TODO: The restart mechanism only persists and loads our own latest unit, so that we don't
// equivocate after a restart. It doesn't yet persist our latest endorsed units, so we could
// accidentally endorse conflicting votes. Fix this and enable slashing for conflicting
// endorsements again.
pub(super) const TODO_ENDORSEMENT_EVIDENCE_DISABLED: bool = true;

/// Number of maximum-length rounds after which a validator counts as offline, if we haven't heard
/// from them.
const PING_TIMEOUT: u64 = 3;

#[derive(Debug, Error, PartialEq, Clone)]
pub(crate) enum UnitError {
    #[error("The unit is a ballot but doesn't cite any block.")]
    MissingBlock,
    #[error("The panorama's length {} doesn't match the number of validators.", _0)]
    PanoramaLength(usize),
    #[error("The unit accuses its own creator as faulty.")]
    FaultyCreator,
    #[error("The previous unit doesn't match the panorama entry.")]
    PreviousUnit,
    #[error("The panorama has a unit from {:?} in the slot for {:?}.", _0, _1)]
    PanoramaIndex(ValidatorIndex, ValidatorIndex),
    #[error("The panorama hash does not match.")]
    PanoramaHash,
    #[error("The panorama is missing units indirectly cited via {:?}.", _0)]
    InconsistentPanorama(ValidatorIndex),
    #[error("Hash-based and sequence number-based panoramas disagree in {:?}.", _0)]
    PanoramaMismatch(ValidatorIndex),
    #[error("The unit contains the wrong sequence number.")]
    SequenceNumber,
    #[error("The unit's timestamp is older than a justification's.")]
    Timestamps,
    #[error("The creator is not a validator.")]
    Creator,
    #[error("The unit was created for a wrong instance ID.")]
    InstanceId,
    #[error("The signature is invalid.")]
    Signature,
    #[error("The round length exponent has somehow changed within a round.")]
    RoundLengthExpChangedWithinRound,
    #[error("The round length exponent is less than the minimum allowed by the chain-spec.")]
    RoundLengthExpLessThanMinimum,
    #[error("The round length exponent is greater than the maximum allowed by the chain-spec.")]
    RoundLengthExpGreaterThanMaximum,
    #[error("This would be the third unit in that round. Only two are allowed.")]
    ThreeUnitsInRound,
    #[error(
        "A block must be the leader's ({:?}) first unit, at the beginning of the round.",
        _0
    )]
    NonLeaderBlock(ValidatorIndex),
    #[error("The unit is a block, but its parent is already a terminal block.")]
    ValueAfterTerminalBlock,
    #[error("The unit's creator is banned.")]
    Banned,
    #[error("The unit's endorsed units were not a superset of its justifications.")]
    EndorsementsNotMonotonic,
    #[error("The LNC rule was violated. Vote cited ({:?}) naively.", _0)]
    LncNaiveCitation(ValidatorIndex),
    #[error(
        "Wire unit endorses hash but does not see it. Hash: {:?}; Wire unit: {:?}",
        hash,
        wire_unit
    )]
    EndorsedButUnseen { hash: String, wire_unit: String },
}

/// A reason for a validator to be marked as faulty.
///
/// The `Banned` state is fixed from the beginning and can't be replaced. However, `Indirect` can
/// be replaced with `Direct` evidence, which has the same effect but doesn't rely on information
/// from other consensus protocol instances.
#[derive(Clone, DataSize, Debug, Eq, PartialEq, Hash)]
pub(crate) enum Fault<C>
where
    C: Context,
{
    /// The validator was known to be malicious from the beginning. All their messages are
    /// considered invalid in this Highway instance.
    Banned,
    /// We have direct evidence of the validator's fault.
    Direct(Evidence<C>),
    /// The validator is known to be faulty, but the evidence is not in this Highway instance.
    Indirect,
}

impl<C: Context> Fault<C> {
    pub(crate) fn evidence(&self) -> Option<&Evidence<C>> {
        match self {
            Fault::Banned | Fault::Indirect => None,
            Fault::Direct(ev) => Some(ev),
        }
    }
}

/// A passive instance of the Highway protocol, containing its local state.
///
/// Both observers and active validators must instantiate this, pass in all incoming vertices from
/// peers, and use a [FinalityDetector](../finality_detector/struct.FinalityDetector.html) to
/// determine the outcome of the consensus process.
#[derive(Debug, Clone, DataSize)]
pub(crate) struct State<C>
where
    C: Context,
{
    /// The fixed parameters.
    params: Params,
    /// The validator's voting weights.
    weights: ValidatorMap<Weight>,
    /// Cumulative validator weights: Entry `i` contains the sum of the weights of validators `0`
    /// through `i`.
    cumulative_w: ValidatorMap<Weight>,
    /// Cumulative validator weights, but with the weight of banned validators set to `0`.
    cumulative_w_leaders: ValidatorMap<Weight>,
    /// All units imported so far, by hash.
    /// This is a downward closed set: A unit must only be added here once all of its dependencies
    /// have been added as well, and it has been fully validated.
    #[data_size(with = ds::hashmap_sample)]
    units: HashMap<C::Hash, Unit<C>>,
    /// All blocks, by hash. All block hashes are also unit hashes, but units that did not
    /// introduce a new block don't have their own entry here.
    #[data_size(with = ds::hashmap_sample)]
    blocks: HashMap<C::Hash, Block<C>>,
    /// List of faulty validators and their type of fault.
    /// Every validator that has an equivocation in `units` must have an entry here, but there can
    /// be additional entries for other kinds of faults.
    faults: HashMap<ValidatorIndex, Fault<C>>,
    /// A map with `true` for all validators that should be assign slots as leaders, and are
    /// allowed to propose blocks.
    can_propose: ValidatorMap<bool>,
    /// The full panorama, corresponding to the complete protocol state.
    /// This points to the latest unit of every honest validator.
    panorama: Panorama<C>,
    /// A unit with maximal sequence number from each validator. For equivocators, this is one of
    /// their forks.
    latest_units: ValidatorMap<Option<C::Hash>>,
    /// All currently endorsed units, by hash: units that have enough endorsements to be cited even
    /// if they naively cite an equivocator.
    #[data_size(with = ds::hashmap_sample)]
    endorsements: HashMap<C::Hash, ValidatorMap<Option<C::Signature>>>,
    /// Units that don't yet have 1/2 of stake endorsing them.
    /// Signatures are stored in a map so that a single validator sending lots of signatures for
    /// different units doesn't cause us to allocate a lot of memory.
    #[data_size(with = ds::hashmap_sample)]
    incomplete_endorsements: HashMap<C::Hash, BTreeMap<ValidatorIndex, C::Signature>>,
    /// Timestamp of the last ping or unit we received from each validator.
    pings: ValidatorMap<Timestamp>,
    /// Clock to measure time spent in fork choice computation.
    #[data_size(skip)] // Not implemented for Clock; probably negligible.
    clock: Clock,
}

impl<C: Context> State<C> {
    pub(crate) fn new<I, IB, IB2>(
        weights: I,
        params: Params,
        banned: IB,
        cannot_propose: IB2,
    ) -> State<C>
    where
        I: IntoIterator,
        I::Item: Borrow<Weight>,
        IB: IntoIterator<Item = ValidatorIndex>,
        IB2: IntoIterator<Item = ValidatorIndex>,
    {
        let weights = ValidatorMap::from(weights.into_iter().map(|w| *w.borrow()).collect_vec());
        assert!(
            weights.len() > 0,
            "cannot initialize Highway with no validators"
        );
        let sums = |mut sums: Vec<Weight>, w: Weight| {
            let sum = sums.last().copied().unwrap_or(Weight(0));
            sums.push(sum.checked_add(w).expect("total weight must be < 2^64"));
            sums
        };
        let cumulative_w = ValidatorMap::from(weights.iter().copied().fold(vec![], sums));
        assert!(
            *cumulative_w.as_ref().last().unwrap() > Weight(0),
            "total weight must not be zero"
        );
        let mut panorama = Panorama::new(weights.len());
        let mut can_propose: ValidatorMap<bool> = weights.iter().map(|_| true).collect();
        for idx in cannot_propose {
            assert!(
                idx.0 < weights.len() as u32,
                "invalid validator index for exclusion from leader sequence"
            );
            can_propose[idx] = false;
        }
        let faults: HashMap<_, _> = banned.into_iter().map(|idx| (idx, Fault::Banned)).collect();
        for idx in faults.keys() {
            assert!(
                idx.0 < weights.len() as u32,
                "invalid banned validator index"
            );
            panorama[*idx] = Observation::Faulty;
        }
        let cumulative_w_leaders = weights
            .enumerate()
            .map(|(idx, weight)| can_propose[idx].then(|| *weight).unwrap_or(Weight(0)))
            .fold(vec![], sums)
            .into();
        let pings = iter::repeat(params.start_timestamp())
            .take(weights.len())
            .collect();
        let latest_units = vec![None; weights.len()].into();
        State {
            params,
            weights,
            cumulative_w,
            cumulative_w_leaders,
            units: HashMap::new(),
            blocks: HashMap::new(),
            faults,
            can_propose,
            panorama,
            latest_units,
            endorsements: HashMap::new(),
            incomplete_endorsements: HashMap::new(),
            pings,
            clock: Clock::new(),
        }
    }

    /// Returns the fixed parameters.
    pub(crate) fn params(&self) -> &Params {
        &self.params
    }

    /// Returns the number of validators.
    pub(crate) fn validator_count(&self) -> usize {
        self.weights.len()
    }

    /// Returns the `idx`th validator's voting weight.
    pub(crate) fn weight(&self, idx: ValidatorIndex) -> Weight {
        self.weights[idx]
    }

    /// Returns the map of validator weights.
    pub(crate) fn weights(&self) -> &ValidatorMap<Weight> {
        &self.weights
    }

    /// Returns the total weight of all validators marked faulty in this panorama.
    pub(crate) fn faulty_weight_in(&self, panorama: &Panorama<C>) -> Weight {
        panorama
            .iter()
            .zip(&self.weights)
            .filter(|(obs, _)| **obs == Observation::Faulty)
            .map(|(_, w)| *w)
            .sum()
    }

    /// Returns the total weight of all known-faulty validators.
    pub(crate) fn faulty_weight(&self) -> Weight {
        self.faulty_weight_in(self.panorama())
    }

    /// Returns the sum of all validators' voting weights.
    pub(crate) fn total_weight(&self) -> Weight {
        *self
            .cumulative_w
            .as_ref()
            .last()
            .expect("weight list cannot be empty")
    }

    /// Returns evidence against validator nr. `idx`, if present.
    pub(crate) fn maybe_evidence(&self, idx: ValidatorIndex) -> Option<&Evidence<C>> {
        self.maybe_fault(idx).and_then(Fault::evidence)
    }

    /// Returns endorsements for `unit`, if any.
    pub(crate) fn maybe_endorsements(&self, unit: &C::Hash) -> Option<Endorsements<C>> {
        self.endorsements.get(unit).map(|signatures| Endorsements {
            unit: *unit,
            endorsers: signatures.iter_some().map(|(i, sig)| (i, *sig)).collect(),
        })
    }

    /// Returns whether evidence against validator nr. `idx` is known.
    pub(crate) fn has_evidence(&self, idx: ValidatorIndex) -> bool {
        self.maybe_evidence(idx).is_some()
    }

    /// Returns whether we have all endorsements for `unit`.
    pub(crate) fn has_all_endorsements<I: IntoIterator<Item = ValidatorIndex>>(
        &self,
        unit: &C::Hash,
        v_ids: I,
    ) -> bool {
        if self.endorsements.contains_key(unit) {
            true // We have enough endorsements for this unit.
        } else if let Some(sigs) = self.incomplete_endorsements.get(unit) {
            v_ids.into_iter().all(|v_id| sigs.contains_key(&v_id))
        } else {
            v_ids.into_iter().next().is_none()
        }
    }

    /// Returns whether we have seen enough endorsements for the unit.
    /// Unit is endorsed when it has endorsements from more than 50% of the validators (by weight).
    pub(crate) fn is_endorsed(&self, hash: &C::Hash) -> bool {
        self.endorsements.contains_key(hash)
    }

    /// Returns hash of unit that needs to be endorsed.
    pub(crate) fn needs_endorsements(&self, unit: &SignedWireUnit<C>) -> Option<C::Hash> {
        unit.wire_unit()
            .endorsed
            .iter()
            .find(|hash| !self.endorsements.contains_key(&hash))
            .cloned()
    }

    /// Computes the full panorama from the sequence number-based one, or returns missing
    /// dependencies.
    pub(crate) fn compute_panorama(
        &self,
        swunit: &SignedWireUnit<C>,
    ) -> Result<Panorama<C>, Dependency<C>> {
        let wunit = swunit.wire_unit();

        // First we request all predecessors of the unit, i.e. wunit.creator's swimlane.
        let _maybe_prev_unit = if let Some(previous) = wunit.previous() {
            if let Some(unit) = self.units.get(previous) {
                Some(unit) // We already have the predecessor and thus the whole swimlane.
            } else {
                // Compute the lowest sequence number that is still missing in our state.
                let maybe_next_seq_num = match self.panorama[wunit.creator] {
                    Observation::Correct(hash) => self.unit(&hash).seq_number.checked_add(1),
                    Observation::None => Some(0),
                    Observation::Faulty => None, // If faulty we always request by hash.
                };
                // If more than one unit is still missing request the earliest one. Otherwise
                // request the predecessor by hash.
                let dep = maybe_next_seq_num
                    .filter(|sn| sn.saturating_add(1) < wunit.seq_number)
                    .map_or_else(
                        || Dependency::Unit(*previous),
                        |seq_num| Dependency::UnitBySeqNum(seq_num, wunit.creator),
                    );
                return Err(dep);
            }
        } else {
            None // This is the first unit by wunit.creator.
        };

        // TODO: Use sequence number _differences_. Compare with maybe_prev_unit.
        // We try to construct the full panorama from the sequence number-based one, or request
        // the missing units if any.
        let panorama = wunit
            .seq_num_panorama
            .enumerate()
            .zip(&self.panorama)
            .map(|((idx, sn_obs), obs)| match (sn_obs, obs) {
                // The incoming unit doesn't cite validator number idx at all.
                (ObservationSeqNum::None, _) => Ok(Observation::None),
                // It cites them as faulty, and we already have evidende.
                (ObservationSeqNum::Faulty, Observation::Faulty) => Ok(Observation::Faulty),
                // It cites them as faulty: We need to request evidence.
                (ObservationSeqNum::Faulty, Observation::Correct(_))
                | (ObservationSeqNum::Faulty, Observation::None) => Err(Dependency::Evidence(idx)),
                // It cites them as correct but we know they're faulty. We don't know which fork to
                // use, so we have to request the full panorama.
                (ObservationSeqNum::Correct(_), Observation::Faulty) => {
                    Err(Dependency::UnitWithPanorama(swunit.hash()))
                }
                // It cites them as correct and we also see them as correct. If we have the cited
                // sequence number we assume it's the right unit. Otherwise we request the cited
                // validator's next unit.
                (ObservationSeqNum::Correct(seq_num), Observation::Correct(hash)) => self
                    .find_in_swimlane(hash, *seq_num)
                    .ok_or_else(|| {
                        let next_seq_num = self.unit(hash).seq_number.saturating_add(1);
                        Dependency::UnitBySeqNum(next_seq_num, idx)
                    })
                    .map(|hash| Observation::Correct(*hash)),
                // It cites them as correct but we don't have any unit from them yet. We request
                // the cited validator's first unit.
                (ObservationSeqNum::Correct(_), Observation::None) => {
                    Err(Dependency::UnitBySeqNum(0, idx))
                }
            })
            .collect::<Result<Panorama<C>, Dependency<C>>>()?;

        // We computed the hash-based panorama. If its hash doesn't match someone must have
        // equivocated whom neither we nor wunit's panorama has marked as faulty. Then we need to
        // download the hash-based panorama from the peer.
        if wunit.panorama_hash == panorama.hash() {
            Ok(panorama)
        } else {
            Err(Dependency::UnitWithPanorama(swunit.hash()))
        }
    }

    /// Returns the timestamp of the last ping or unit received from the validator, or the start
    /// timestamp if we haven't received anything yet.
    pub(crate) fn last_seen(&self, idx: ValidatorIndex) -> Timestamp {
        self.pings[idx]
    }

    /// Marks the given validator as faulty, unless it is already banned or we have direct evidence.
    pub(crate) fn mark_faulty(&mut self, idx: ValidatorIndex) {
        self.panorama[idx] = Observation::Faulty;
        self.faults.entry(idx).or_insert(Fault::Indirect);
    }

    /// Returns the fault type of validator nr. `idx`, if it is known to be faulty.
    pub(crate) fn maybe_fault(&self, idx: ValidatorIndex) -> Option<&Fault<C>> {
        self.faults.get(&idx)
    }

    /// Returns whether validator nr. `idx` is known to be faulty.
    pub(crate) fn is_faulty(&self, idx: ValidatorIndex) -> bool {
        self.faults.contains_key(&idx)
    }

    /// Returns an iterator over all faulty validators.
    pub(crate) fn faulty_validators(&self) -> impl Iterator<Item = ValidatorIndex> + '_ {
        self.faults.keys().cloned()
    }

    /// Returns an iterator over latest unit hashes from honest validators.
    pub(crate) fn iter_correct_hashes(&self) -> impl Iterator<Item = &C::Hash> {
        self.panorama.iter_correct_hashes()
    }

    /// Returns the unit with the given hash, if present.
    pub(crate) fn maybe_unit(&self, hash: &C::Hash) -> Option<&Unit<C>> {
        self.units.get(hash)
    }

    /// Returns the hash of a unit with maximal sequence number by the given validator.
    pub(crate) fn maybe_latest_unit(&self, vidx: ValidatorIndex) -> Option<&C::Hash> {
        self.latest_units
            .has(vidx)
            .then(|| self.latest_units.get(vidx).as_ref())
            .flatten()
    }

    /// Returns whether the unit with the given hash is known.
    pub(crate) fn has_unit(&self, hash: &C::Hash) -> bool {
        self.units.contains_key(hash)
    }

    /// Returns the unit with the given hash. Panics if not found.
    pub(crate) fn unit(&self, hash: &C::Hash) -> &Unit<C> {
        self.maybe_unit(hash).expect("unit hash must exist")
    }

    /// Returns the block contained in the unit with the given hash, if present.
    pub(crate) fn maybe_block(&self, hash: &C::Hash) -> Option<&Block<C>> {
        self.blocks.get(hash)
    }

    /// Returns the block contained in the unit with the given hash. Panics if not found.
    pub(crate) fn block(&self, hash: &C::Hash) -> &Block<C> {
        self.maybe_block(hash).expect("block hash must exist")
    }

    /// Returns the complete protocol state's latest panorama.
    pub(crate) fn panorama(&self) -> &Panorama<C> {
        &self.panorama
    }

    /// Returns the leader in the specified time slot.
    ///
    /// First the assignment is computed ignoring the `can_propose` flags. Only if the selected
    /// leader's entry is `false`, the computation is repeated, this time with the flagged
    /// validators excluded. This ensures that once the validator set has been decided, correct
    /// validators' slots never get reassigned to someone else, even if after the fact someone is
    /// excluded as a leader.
    pub(crate) fn leader(&self, timestamp: Timestamp) -> ValidatorIndex {
        // The binary search cannot return None; if it does, it's a programming error. In that case,
        // we want the tests to panic but production to pick a default.
        let panic_or_0 = || {
            if cfg!(test) {
                panic!("random number out of range");
            } else {
                error!("random number out of range");
                ValidatorIndex(0)
            }
        };
        let seed = self.params.seed().wrapping_add(timestamp.millis());
        // We select a random one out of the `total_weight` weight units, starting numbering at 1.
        let r = Weight(leader_prng(self.total_weight().0, seed));
        // The weight units are subdivided into intervals that belong to some validator.
        // `cumulative_w[i]` denotes the last weight unit that belongs to validator `i`.
        // `binary_search` returns the first `i` with `cumulative_w[i] >= r`, i.e. the validator
        // who owns the randomly selected weight unit.
        let leader_index = self
            .cumulative_w
            .binary_search(&r)
            .unwrap_or_else(panic_or_0);
        if self.can_propose[leader_index] {
            return leader_index;
        }
        // If the selected leader is excluded, we reassign the slot to someone else. This time we
        // consider only the non-banned validators.
        let total_w_leaders = *self.cumulative_w_leaders.as_ref().last().unwrap();
        let r = Weight(leader_prng(total_w_leaders.0, seed.wrapping_add(1)));
        self.cumulative_w_leaders
            .binary_search(&r)
            .unwrap_or_else(panic_or_0)
    }

    /// Adds the unit to the protocol state.
    ///
    /// The unit must be valid (see `validate_unit`), and its dependencies satisfied.
    pub(crate) fn add_valid_unit(&mut self, swunit: SignedWireUnit<C>, panorama: Panorama<C>) {
        let wunit = swunit.wire_unit();
        let hash = swunit.hash();
        if self.has_unit(&hash) {
            warn!(%hash, "called add_valid_unit twice");
            return;
        }
        let instance_id = wunit.instance_id;
        let fork_choice = self.fork_choice(&panorama).cloned();
        let (unit, maybe_value) = Unit::new(swunit, panorama, fork_choice.as_ref(), self);
        if let Some(value) = maybe_value {
            let block = Block::new(fork_choice, value, self);
            self.blocks.insert(hash, block);
        }
        self.add_ping(unit.creator, unit.timestamp);
        self.units.insert(hash, unit);

        let unit = self.unit(&hash);
        let creator = unit.creator;
        let seq_number = unit.seq_number;

        // Update the panorama.
        let new_obs = match (self.panorama.get(creator), unit.panorama.get(creator)) {
            (Observation::Faulty, _) => Observation::Faulty,
            (obs0, obs1) if obs0 == obs1 => Observation::Correct(hash),
            (Observation::None, _) => panic!("missing creator's previous unit"),
            (Observation::Correct(hash0), _) => {
                // We have all dependencies of unit, but it does not cite hash0 as its predecessor,
                // which is our latest known unit by the creator. It must therefore cite an older
                // unit and so its sequence number must be at most the same as hash0. Hence it is
                // an equivocation, and to prove that, we only need to provide the other unit with
                // the same sequence number.
                let prev0 = self.find_in_swimlane(hash0, unit.seq_number).unwrap();
                let wunit0 = self.wire_unit(prev0, instance_id).unwrap();
                let wunit1 = self.wire_unit(&hash, instance_id).unwrap();
                self.add_evidence(Evidence::Equivocation(wunit0, wunit1));
                Observation::Faulty
            }
        };
        self.panorama[creator] = new_obs;

        // Update the map of latest units.
        if self.latest_units[creator]
            .filter(|hash| self.unit(hash).seq_number >= seq_number)
            .is_none()
        {
            self.latest_units[creator] = Some(hash);
        }
    }

    /// Adds direct evidence proving a validator to be faulty, unless that validators is already
    /// banned or we already have other direct evidence. This must only be called with valid
    /// evidence (see `Evidence::validate`). Returns `false` if the evidence was not added because
    /// the perpetrator is banned or we already have evidence against them.
    pub(crate) fn add_evidence(&mut self, evidence: Evidence<C>) -> bool {
        if TODO_ENDORSEMENT_EVIDENCE_DISABLED && matches!(evidence, Evidence::Endorsements { .. }) {
            return false;
        }
        let idx = evidence.perpetrator();
        match self.faults.get(&idx) {
            Some(&Fault::Banned) | Some(&Fault::Direct(_)) => return false,
            None | Some(&Fault::Indirect) => (),
        }
        // TODO: Should use Display, not Debug!
        trace!(?evidence, "marking validator #{} as faulty", idx.0);
        self.faults.insert(idx, Fault::Direct(evidence));
        self.panorama[idx] = Observation::Faulty;
        true
    }

    /// Adds a set of endorsements to the state.
    /// If, after adding, we have collected enough endorsements to consider unit _endorsed_,
    /// it will be *upgraded* to fully endorsed.
    ///
    /// Endorsements must be validated before calling this: The endorsers must exist, the
    /// signatures must be valid and the endorsed unit must be present in `self.units`.
    pub(crate) fn add_endorsements(&mut self, endorsements: Endorsements<C>) {
        let uhash = *endorsements.unit();
        if self.endorsements.contains_key(&uhash) {
            return; // We already have a sufficient number of endorsements.
        }
        info!("Received endorsements of {:?}", uhash);
        self.incomplete_endorsements
            .entry(uhash)
            .or_default()
            .extend(endorsements.endorsers);
        let endorsed: Weight = self.incomplete_endorsements[&uhash]
            .keys()
            .map(|vidx| self.weight(*vidx))
            .sum();
        // Stake required to consider unit to be endorsed.
        let threshold = self.total_weight() / 2;
        if endorsed > threshold {
            info!(%uhash, "Unit endorsed by at least 1/2 of validators.");
            // Unwrap is safe: We created the map entry above.
            let mut fully_endorsed = self.incomplete_endorsements.remove(&uhash).unwrap();
            let endorsed_map = self
                .weights()
                .keys()
                .map(|vidx| fully_endorsed.remove(&vidx))
                .collect();
            self.endorsements.insert(uhash, endorsed_map);
        }
    }

    /// Returns whether this state already includes an endorsement of `uhash` by `vidx`.
    pub(crate) fn has_endorsement(&self, uhash: &C::Hash, vidx: ValidatorIndex) -> bool {
        self.endorsements
            .get(uhash)
            .map(|vmap| vmap[vidx].is_some())
            .unwrap_or(false)
            || self
                .incomplete_endorsements
                .get(uhash)
                .map(|ends| ends.contains_key(&vidx))
                .unwrap_or(false)
    }

    /// Updates `self.pings` with the given timestamp.
    pub(crate) fn add_ping(&mut self, creator: ValidatorIndex, timestamp: Timestamp) {
        self.pings[creator] = self.pings[creator].max(timestamp);
    }

    /// Returns `true` if the latest timestamp we have is less than one maximum round length older
    /// than the given timestamp.
    ///
    /// This is to prevent ping spam: If the incoming ping is only slightly newer than a unit or
    /// ping we have already received, we drop it without forwarding it to our peers.
    pub(crate) fn has_ping(&self, creator: ValidatorIndex, timestamp: Timestamp) -> bool {
        self.pings.has(creator) && self.pings[creator] + self.params.max_round_length() > timestamp
    }

    /// Returns whether the validator's latest unit or ping is at most `PING_TIMEOUT` maximum round
    /// lengths old.
    pub(crate) fn is_online(&self, vidx: ValidatorIndex, now: Timestamp) -> bool {
        self.pings.has(vidx)
            && self.pings[vidx] + self.params.max_round_length() * PING_TIMEOUT >= now
    }

    /// Creates new `Evidence` if the new endorsements contain any that conflict with existing
    /// ones.
    ///
    /// Endorsements must be validated before calling this: The endorsers must exist, the
    /// signatures must be valid and the endorsed unit must be present in `self.units`.
    pub(crate) fn find_conflicting_endorsements(
        &self,
        endorsements: &Endorsements<C>,
        instance_id: &C::InstanceId,
    ) -> Vec<Evidence<C>> {
        if TODO_ENDORSEMENT_EVIDENCE_DISABLED {
            return Vec::new();
        }
        let uhash = endorsements.unit();
        let unit = self.unit(&uhash);
        if !self.has_evidence(unit.creator) {
            return vec![]; // There are no equivocations, so endorsements cannot conflict.
        }

        // We are only interested in endorsements that we didn't know before and whose validators
        // we don't already have evidence against.
        let is_new_endorsement = |&&(vidx, _): &&(ValidatorIndex, _)| {
            if self.has_evidence(vidx) {
                false
            } else if let Some(known_endorsements) = self.endorsements.get(&uhash) {
                known_endorsements[vidx].is_none()
            } else if let Some(known_endorsements) = self.incomplete_endorsements.get(&uhash) {
                !known_endorsements.contains_key(&vidx)
            } else {
                true
            }
        };
        let new_endorsements = endorsements.endorsers.iter().filter(is_new_endorsement);

        // For each new endorser, find a pair of conflicting endorsements, if it exists.
        // Order the data so that the first unit has a lower or equal sequence number.
        let conflicting_endorsements = new_endorsements.filter_map(|&(vidx, ref sig)| {
            // Iterate over all existing endorsements by vidx.
            let known_endorsements = self.endorsements.iter();
            let known_incomplete_endorsements = self.incomplete_endorsements.iter();
            let known_vidx_endorsements = known_endorsements
                .filter_map(|(uhash2, sigs2)| sigs2[vidx].as_ref().map(|sig2| (uhash2, sig2)));
            let known_vidx_incomplete_endorsements = known_incomplete_endorsements
                .filter_map(|(uhash2, sigs2)| sigs2.get(&vidx).map(|sig2| (uhash2, sig2)));
            // Find a conflicting one, i.e. one that endorses a unit with the same creator as uhash
            // but incompatible with uhash. Put the data into a tuple, with the earlier unit first.
            known_vidx_endorsements
                .chain(known_vidx_incomplete_endorsements)
                .find(|(uhash2, _)| {
                    let unit2 = self.unit(uhash2);
                    let ee_limit = self.params().endorsement_evidence_limit();
                    self.unit(uhash2).creator == unit.creator
                        && !self.is_compatible(&uhash, uhash2)
                        && unit.seq_number.saturating_add(ee_limit) >= unit2.seq_number
                        && unit2.seq_number.saturating_add(ee_limit) >= unit.seq_number
                })
                .map(|(uhash2, sig2)| {
                    if unit.seq_number <= self.unit(uhash2).seq_number {
                        (vidx, uhash, sig, uhash2, sig2)
                    } else {
                        (vidx, uhash2, sig2, uhash, sig)
                    }
                })
        });

        // Create an Evidence instance for each conflict we found.
        // The unwraps are safe, because we know that there are units with these hashes.
        conflicting_endorsements
            .map(|(vidx, uhash1, sig1, uhash2, sig2)| {
                let unit1 = self.unit(uhash1);
                let swimlane2 = self
                    .swimlane(uhash2)
                    .skip(1)
                    .take_while(|(_, pred2)| pred2.seq_number >= unit1.seq_number)
                    .map(|(pred2_hash, _)| self.wire_unit(pred2_hash, *instance_id).unwrap())
                    .collect();
                Evidence::Endorsements {
                    endorsement1: SignedEndorsement::new(Endorsement::new(*uhash1, vidx), *sig1),
                    unit1: self.wire_unit(uhash1, *instance_id).unwrap(),
                    endorsement2: SignedEndorsement::new(Endorsement::new(*uhash2, vidx), *sig2),
                    unit2: self.wire_unit(uhash2, *instance_id).unwrap(),
                    swimlane2,
                }
            })
            .collect()
    }

    /// Returns the `SignedWireUnit` with the given hash, if it is present in the state.
    pub(crate) fn wire_unit(
        &self,
        hash: &C::Hash,
        instance_id: C::InstanceId,
    ) -> Option<SignedWireUnit<C>> {
        let unit = self.maybe_unit(hash)?.clone();
        let maybe_block = self.maybe_block(hash);
        let value = maybe_block.map(|block| block.value.clone());
        let endorsed = unit.claims_endorsed().cloned().collect();
        let wunit = WireUnit {
            seq_num_panorama: unit.panorama.to_seq_num_panorama(&self),
            panorama_hash: unit.panorama_hash,
            previous: unit.panorama[unit.creator].correct().cloned(),
            creator: unit.creator,
            instance_id,
            value,
            seq_number: unit.seq_number,
            timestamp: unit.timestamp,
            round_exp: unit.round_exp,
            endorsed,
        };
        Some(SignedWireUnit {
            hashed_wire_unit: HashedWireUnit::new_with_hash(wunit, *hash),
            signature: unit.signature,
        })
    }

    /// Returns the fork choice from `pan`'s view, or `None` if there are no blocks yet.
    ///
    /// The correct validators' latest units count as votes for the block they point to, as well as
    /// all of its ancestors. At each level the block with the highest score is selected from the
    /// children of the previously selected block (or from all blocks at height 0), until a block
    /// is reached that has no children with any votes.
    pub(crate) fn fork_choice<'a>(&'a self, pan: &Panorama<C>) -> Option<&'a C::Hash> {
        let start = self.clock.start();
        // Collect all correct votes in a `Tallies` map, sorted by height.
        let to_entry = |(obs, w): (&Observation<C>, &Weight)| {
            let bhash = &self.unit(obs.correct()?).block;
            Some((self.block(bhash).height, bhash, *w))
        };
        let mut tallies: Tallies<C> = pan.iter().zip(&self.weights).filter_map(to_entry).collect();
        loop {
            // Find the highest block that we know is an ancestor of the fork choice.
            let (height, bhash) = tallies.find_decided(self)?;
            // Drop all votes that are not descendants of `bhash`.
            tallies = tallies.filter_descendants(height, bhash, self);
            // If there are no blocks left, `bhash` itself is the fork choice. Otherwise repeat.
            if tallies.is_empty() {
                let end = self.clock.end();
                let delta = self.clock.delta(start, end).as_nanos();
                trace!(%delta,"Time taken for fork-choice to run");
                return Some(bhash);
            }
        }
    }

    /// Returns the ancestor of the block with the given `hash`, on the specified `height`, or
    /// `None` if the block's height is lower than that.
    pub(crate) fn find_ancestor<'a>(
        &'a self,
        hash: &'a C::Hash,
        height: u64,
    ) -> Option<&'a C::Hash> {
        let block = self.block(hash);
        if block.height < height {
            return None;
        }
        if block.height == height {
            return Some(hash);
        }
        #[allow(clippy::integer_arithmetic)] // block.height > height, otherwise we returned.
        let diff = block.height - height;
        // We want to make the greatest step 2^i such that 2^i <= diff.
        let max_i = log2(diff) as usize;
        // A block at height > 0 always has at least its parent entry in skip_idx.
        #[allow(clippy::integer_arithmetic)]
        let i = max_i.min(block.skip_idx.len() - 1);
        self.find_ancestor(&block.skip_idx[i], height)
    }

    /// Returns an error if `swunit` is invalid. This can be called even if the dependencies are
    /// not present yet.
    pub(crate) fn pre_validate_unit(
        &self,
        swunit: &SignedWireUnit<C>,
        maybe_panorama: Option<&Panorama<C>>,
    ) -> Result<(), UnitError> {
        let wunit = swunit.wire_unit();
        let creator = wunit.creator;

        // Check that the unit's creator exists and isn't banned, and …
        if creator.0 as usize >= self.validator_count() {
            error!("Nonexistent validator should be rejected in Highway::pre_validate_unit.");
            return Err(UnitError::Creator); // Should be unreachable.
        }
        if Some(&Fault::Banned) == self.faults.get(&creator) {
            return Err(UnitError::Banned);
        }

        // …that the round length is valid.
        if wunit.round_exp < self.params.min_round_exp() {
            return Err(UnitError::RoundLengthExpLessThanMinimum);
        }
        if wunit.round_exp > self.params.max_round_exp() {
            return Err(UnitError::RoundLengthExpGreaterThanMaximum);
        }

        // A unit that does not cite any other unit must propose a value.
        if wunit.value.is_none()
            && !wunit
                .seq_num_panorama
                .iter()
                .any(ObservationSeqNum::is_correct)
        {
            return Err(UnitError::MissingBlock);
        }

        // The panorama must have exactly one entry for each validator.
        if wunit.seq_num_panorama.len() != self.validator_count() {
            return Err(UnitError::PanoramaLength(wunit.seq_num_panorama.len()));
        }

        // The unit's sequence number and predecessor hash must match the panorama.
        match (wunit.seq_num_panorama.get(creator), wunit.previous) {
            // You can't cite yourself as faulty.
            (ObservationSeqNum::Faulty, _) => return Err(UnitError::FaultyCreator),
            // If there is a previous unit it should also show up in the panorama, and vice versa.
            (ObservationSeqNum::None, Some(_)) | (ObservationSeqNum::Correct(_), None) => {
                return Err(UnitError::PreviousUnit);
            }
            // This unit's sequence number must be exactly one more than its predecessor's.
            (ObservationSeqNum::Correct(seq_num), Some(_)) => {
                if seq_num.checked_add(1) != Some(wunit.seq_number) {
                    return Err(UnitError::SequenceNumber);
                }
            }
            // If it is its creator's first unit, it must have sequence number zero.
            (ObservationSeqNum::None, None) if wunit.seq_number != 0 => {
                return Err(UnitError::SequenceNumber);
            }
            _ => {}
        }

        // If we have the hash-based panorama, it must match the predecessor hash and the
        // sequence number-based panorama.
        if let Some(panorama) = maybe_panorama {
            if panorama.len() != self.validator_count() {
                return Err(UnitError::PanoramaLength(panorama.len()));
            }
            if panorama.get(creator).correct() != wunit.previous.as_ref() {
                return Err(UnitError::PreviousUnit);
            }
            for ((vidx, obs), sn_obs) in panorama.enumerate().zip(&wunit.seq_num_panorama) {
                match (obs, sn_obs) {
                    (Observation::Correct(_), ObservationSeqNum::Correct(_))
                    | (Observation::None, ObservationSeqNum::None)
                    | (Observation::Faulty, ObservationSeqNum::Faulty) => {}
                    (_, _) => return Err(UnitError::PanoramaMismatch(vidx)),
                }
            }
            if panorama.hash() != wunit.panorama_hash {
                return Err(UnitError::PanoramaHash);
            }
        }
        Ok(())
    }

    /// Returns an error if `swunit` is invalid. Must only be called once `pre_validate_unit`
    /// returned `Ok` and all dependencies have been added to the state.
    pub(crate) fn validate_unit(
        &self,
        swunit: &SignedWireUnit<C>,
        panorama: &Panorama<C>,
    ) -> Result<(), UnitError> {
        let wunit = swunit.wire_unit();
        let creator = wunit.creator;
        let timestamp = wunit.timestamp;
        panorama.validate(self)?;
        if panorama.iter_correct(self).any(|v| v.timestamp > timestamp)
            || wunit.timestamp < self.params.start_timestamp()
        {
            return Err(UnitError::Timestamps);
        }
        for ((vidx, obs), seq_obs) in panorama.enumerate().zip(&wunit.seq_num_panorama) {
            match (obs, seq_obs) {
                (Observation::Faulty, ObservationSeqNum::Faulty)
                | (Observation::None, ObservationSeqNum::None) => {}
                (Observation::Correct(hash), ObservationSeqNum::Correct(seq_number))
                    if self.unit(hash).seq_number == *seq_number => {}
                (_, _) => return Err(UnitError::PanoramaMismatch(vidx)),
            }
        }
        if wunit.seq_number != panorama.next_seq_num(self, creator) {
            return Err(UnitError::SequenceNumber);
        }
        let r_id = round_id(timestamp, wunit.round_exp);
        let maybe_prev_unit = wunit.previous().map(|vh| self.unit(vh));
        if let Some(prev_unit) = maybe_prev_unit {
            if prev_unit.round_exp != wunit.round_exp {
                // The round exponent must not change within a round: Even with respect to the
                // greater of the two exponents, a round boundary must be between the units.
                let max_re = prev_unit.round_exp.max(wunit.round_exp);
                if prev_unit.timestamp >> max_re == timestamp >> max_re {
                    return Err(UnitError::RoundLengthExpChangedWithinRound);
                }
            }
            // There can be at most two units per round: proposal/confirmation and witness.
            if let Some(prev2_unit) = prev_unit.previous().map(|h2| self.unit(h2)) {
                if prev2_unit.round_id() == r_id {
                    return Err(UnitError::ThreeUnitsInRound);
                }
            }
        }
        // All endorsed units from the panorama of this wunit.
        let endorsements_in_panorama = panorama
            .iter_correct_hashes()
            .flat_map(|hash| self.unit(hash).claims_endorsed())
            .collect::<HashSet<_>>();
        if endorsements_in_panorama
            .iter()
            .any(|&e| !wunit.endorsed.iter().any(|h| h == e))
        {
            return Err(UnitError::EndorsementsNotMonotonic);
        }
        if wunit.value.is_some() {
            // If this unit is a block, it must be the first unit in this round, its timestamp must
            // match the round ID, and the creator must be the round leader.
            if maybe_prev_unit.map_or(false, |pv| pv.round_id() == r_id)
                || timestamp != r_id
                || self.leader(r_id) != creator
            {
                return Err(UnitError::NonLeaderBlock(self.leader(r_id)));
            }
            // It's not allowed to create a child block of a terminal block.
            let is_terminal = |hash: &C::Hash| self.is_terminal_block(hash);
            if self.fork_choice(panorama).map_or(false, is_terminal) {
                return Err(UnitError::ValueAfterTerminalBlock);
            }
        }
        for hash in &wunit.endorsed {
            if !panorama.sees(self, hash) {
                return Err(UnitError::EndorsedButUnseen {
                    hash: format!("{:?}", hash),
                    wire_unit: format!("{:?}", wunit),
                });
            }
        }
        match self.validate_lnc(creator, panorama, &wunit.endorsed) {
            None => Ok(()),
            Some(vidx) => Err(UnitError::LncNaiveCitation(vidx)),
        }
    }

    /// Returns `true` if the `bhash` is a block that can have no children.
    pub(crate) fn is_terminal_block(&self, bhash: &C::Hash) -> bool {
        self.blocks.get(bhash).map_or(false, |block| {
            block.height.saturating_add(1) >= self.params.end_height()
                && self.unit(bhash).timestamp >= self.params.end_timestamp()
        })
    }

    /// Returns `true` if this is a proposal and the creator is not faulty.
    pub(super) fn is_correct_proposal(&self, unit: &Unit<C>) -> bool {
        !self.is_faulty(unit.creator)
            && self.leader(unit.timestamp) == unit.creator
            && unit.timestamp == round_id(unit.timestamp, unit.round_exp)
    }

    /// Returns the hash of the message with the given sequence number from the creator of `hash`,
    /// or `None` if the sequence number is higher than that of the unit with `hash`.
    pub(super) fn find_in_swimlane<'a>(
        &'a self,
        hash: &'a C::Hash,
        seq_number: u64,
    ) -> Option<&'a C::Hash> {
        let unit = self.unit(hash);
        match unit.seq_number.checked_sub(seq_number) {
            None => None,          // There is no unit with seq_number in our swimlane.
            Some(0) => Some(hash), // The sequence number is the one we're looking for.
            Some(diff) => {
                // We want to make the greatest step 2^i such that 2^i <= diff.
                let max_i = log2(diff) as usize; // Log is safe because diff is not zero.

                // Diff is not zero, so the unit has a predecessor and skip_idx is not empty.
                #[allow(clippy::integer_arithmetic)]
                let i = max_i.min(unit.skip_idx.len() - 1);
                self.find_in_swimlane(&unit.skip_idx[i], seq_number)
            }
        }
    }

    /// Returns an iterator over units (with hashes) by the same creator, in reverse chronological
    /// order, starting with the specified unit. Panics if no unit with `uhash` exists.
    pub(crate) fn swimlane<'a>(
        &'a self,
        uhash: &'a C::Hash,
    ) -> impl Iterator<Item = (&'a C::Hash, &'a Unit<C>)> {
        let mut next = Some(uhash);
        iter::from_fn(move || {
            let current = next?;
            let unit = self.unit(current);
            next = unit.previous();
            Some((current, unit))
        })
    }

    /// Returns an iterator over all hashes of ancestors of the block `bhash`, excluding `bhash`
    /// itself. Panics if `bhash` is not the hash of a known block.
    pub(crate) fn ancestor_hashes<'a>(
        &'a self,
        bhash: &'a C::Hash,
    ) -> impl Iterator<Item = &'a C::Hash> {
        let mut next = self.block(bhash).parent();
        iter::from_fn(move || {
            let current = next?;
            next = self.block(current).parent();
            Some(current)
        })
    }

    /// Returns `true` if the state contains no units.
    pub(crate) fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Returns the number of units received.
    #[cfg(test)]
    pub(crate) fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// Returns the set of units (by hash) that are endorsed and seen from the panorama.
    pub(crate) fn seen_endorsed(&self, pan: &Panorama<C>) -> BTreeSet<C::Hash> {
        // First we collect all units that were already seen as endorsed by earlier units.
        let mut result: BTreeSet<C::Hash> = pan
            .iter_correct_hashes()
            .flat_map(|hash| self.unit(hash).endorsed.iter().cloned())
            .collect();
        // Now add all remaining endorsed units. Since the pan.sees check is expensive, do it only
        // for the ones that are actually new.
        for hash in self.endorsements.keys() {
            if !result.contains(hash) && pan.sees(self, hash) {
                result.insert(*hash);
            }
        }
        result
    }

    /// Drops all state other than evidence.
    pub(crate) fn retain_evidence_only(&mut self) {
        self.units.clear();
        self.blocks.clear();
        for obs in self.panorama.iter_mut() {
            if obs.is_correct() {
                *obs = Observation::None;
            }
        }
        self.latest_units = vec![None; self.validator_count()].into();
        self.endorsements.clear();
        self.incomplete_endorsements.clear();
    }

    /// Validates whether a unit with the given panorama and `endorsed` set satisfies the
    /// Limited Naïveté Criterion (LNC).
    /// Returns index of the first equivocator that was cited naively in violation of the LNC, or
    /// `None` if the LNC is satisfied.
    fn validate_lnc(
        &self,
        creator: ValidatorIndex,
        panorama: &Panorama<C>,
        endorsed: &BTreeSet<C::Hash>,
    ) -> Option<ValidatorIndex> {
        let violates_lnc =
            |eq_idx: &ValidatorIndex| !self.satisfies_lnc_for(creator, panorama, endorsed, *eq_idx);
        panorama.iter_faulty().find(violates_lnc)
    }

    /// Returns `true` if there is at most one fork by the validator `eq_idx` that is cited naively
    /// by a unit with the given panorama and `endorsed` set, or earlier units by the same creator.
    fn satisfies_lnc_for(
        &self,
        creator: ValidatorIndex,
        panorama: &Panorama<C>,
        endorsed: &BTreeSet<C::Hash>,
        eq_idx: ValidatorIndex,
    ) -> bool {
        // Find all forks by eq_idx that are cited naively by the panorama itself.
        // * If it's more than one, return false: the LNC is violated.
        // * If it's none, return true: If the LNC were violated, it would be because of two naive
        //   citations by creator's earlier units. So the latest of those earlier units would
        //   already be violating the LNC itself, and thus would not have been added to the state.
        // * Otherwise store the unique naively cited fork in naive_fork.
        let mut maybe_naive_fork = None;
        {
            // Returns true if any endorsed unit cites the given unit.
            let seen_by_endorsed = |hash| endorsed.iter().any(|e_hash| self.sees(e_hash, hash));

            // Iterate over all units cited by the panorama.
            let mut to_visit: Vec<_> = panorama.iter_correct_hashes().collect();
            // This set is a filter so that units don't get added to to_visit twice.
            let mut added_to_to_visit: HashSet<_> = to_visit.iter().cloned().collect();
            while let Some(hash) = to_visit.pop() {
                if seen_by_endorsed(hash) {
                    continue; // This unit and everything below is not cited naively.
                }
                let unit = self.unit(hash);
                match &unit.panorama[eq_idx] {
                    Observation::Correct(eq_hash) => {
                        // The unit (and everything it cites) can only see a single fork.
                        // No need to traverse further downward.
                        if !seen_by_endorsed(eq_hash) {
                            // The fork is cited naively!
                            match maybe_naive_fork {
                                // It's the first naively cited fork we found.
                                None => maybe_naive_fork = Some(eq_hash),
                                Some(other_hash) => {
                                    // If eq_hash is later than other_hash, it is the tip of the
                                    // same fork. If it is earlier, then other_hash is the tip.
                                    if self.sees_correct(eq_hash, other_hash) {
                                        maybe_naive_fork = Some(eq_hash);
                                    } else if !self.sees_correct(other_hash, eq_hash) {
                                        return false; // We found two incompatible forks!
                                    }
                                }
                            }
                        }
                    }
                    // No forks are cited by this unit. No need to traverse further.
                    Observation::None => (),
                    // The unit still sees the equivocator as faulty: We need to traverse further
                    // down the graph to find all cited forks.
                    Observation::Faulty => to_visit.extend(
                        unit.panorama
                            .iter_correct_hashes()
                            .filter(|hash| added_to_to_visit.insert(hash)),
                    ),
                }
            }
        }
        let naive_fork = match maybe_naive_fork {
            None => return true, // No forks are cited naively.
            Some(naive_fork) => naive_fork,
        };

        // Iterate over all earlier units by creator, and find all forks by eq_idx they
        // naively cite. If any of those forks are incompatible with naive_fork, the LNC is
        // violated.
        let mut maybe_pred_hash = panorama[creator].correct();
        while let Some(pred_hash) = maybe_pred_hash {
            let pred_unit = self.unit(pred_hash);
            // Returns true if any endorsed (according to pred_unit) unit cites the given unit.
            let seen_by_endorsed = |hash| {
                pred_unit
                    .endorsed
                    .iter()
                    .any(|e_hash| self.sees(e_hash, hash))
            };
            // Iterate over all units seen by pred_unit.
            let mut to_visit = vec![pred_hash];
            // This set is a filter so that units don't get added to to_visit twice.
            let mut added_to_to_visit: HashSet<_> = to_visit.iter().cloned().collect();
            while let Some(hash) = to_visit.pop() {
                if seen_by_endorsed(hash) {
                    continue; // This unit and everything below is not cited naively.
                }
                let unit = self.unit(hash);
                match &unit.panorama[eq_idx] {
                    Observation::Correct(eq_hash) => {
                        if !seen_by_endorsed(eq_hash) && !self.is_compatible(eq_hash, &naive_fork) {
                            return false;
                        }
                    }
                    // No forks are cited by this unit. No need to traverse further.
                    Observation::None => (),
                    // The unit still sees the equivocator as faulty: We need to traverse further
                    // down the graph to find all cited forks.
                    Observation::Faulty => to_visit.extend(
                        unit.panorama
                            .iter_correct_hashes()
                            .filter(|hash| added_to_to_visit.insert(hash)),
                    ),
                }
            }
            if !pred_unit.panorama[eq_idx].is_faulty() {
                // This unit and everything below sees only a single fork of the equivocator. If we
                // haven't found conflicting naively cited forks yet, there are none.
                return true;
            }
            maybe_pred_hash = pred_unit.previous();
        }
        true // No earlier messages, so no conflicting naively cited forks.
    }

    /// Returns whether the unit with `hash0` sees the one with `hash1` (i.e. `hash0 ≥ hash1`),
    /// and sees `hash1`'s creator as correct.
    pub(crate) fn sees_correct(&self, hash0: &C::Hash, hash1: &C::Hash) -> bool {
        hash0 == hash1 || self.unit(hash0).panorama.sees_correct(self, hash1)
    }

    /// Returns whether the unit with `hash0` sees the one with `hash1` (i.e. `hash0 ≥ hash1`).
    pub(crate) fn sees(&self, hash0: &C::Hash, hash1: &C::Hash) -> bool {
        hash0 == hash1 || self.unit(hash0).panorama.sees(self, hash1)
    }

    // Returns whether the units with `hash0` and `hash1` see each other or are equal.
    fn is_compatible(&self, hash0: &C::Hash, hash1: &C::Hash) -> bool {
        hash0 == hash1
            || self.unit(hash0).panorama.sees(self, hash1)
            || self.unit(hash1).panorama.sees(self, hash0)
    }

    /// Returns the panorama of the confirmation for the leader unit `uhash`.
    pub(crate) fn confirmation_panorama(
        &self,
        creator: ValidatorIndex,
        uhash: &C::Hash,
    ) -> Panorama<C> {
        self.valid_panorama(creator, self.inclusive_panorama(uhash))
    }

    /// Creates a panorama that is valid for use in `creator`'s next unit, and as close as possible
    /// to the given one. It is only modified if necessary for validity:
    /// * Cite `creator`'s previous unit, i.e. don't equivocate.
    /// * Satisfy the LNC, i.e. don't add new naively cited forks.
    pub(crate) fn valid_panorama(
        &self,
        creator: ValidatorIndex,
        mut pan: Panorama<C>,
    ) -> Panorama<C> {
        // Make sure the panorama sees the creator's own previous unit.
        let maybe_prev_uhash = self.panorama()[creator].correct();
        if let Some(prev_uhash) = maybe_prev_uhash {
            if pan[creator].correct() != Some(prev_uhash) {
                pan = pan.merge(self, &self.inclusive_panorama(prev_uhash));
            }
        }
        let endorsed = self.seen_endorsed(&pan);
        if self.validate_lnc(creator, &pan, &endorsed).is_none() {
            return pan;
        }
        // `pan` violates the LNC.
        // Start from the creator's previous unit, mark all faulty
        // validators as faulty, and add only endorsed units from correct validators.
        pan = maybe_prev_uhash.map_or_else(
            || Panorama::new(self.validator_count()),
            |prev_uhash| self.inclusive_panorama(prev_uhash),
        );
        for faulty_v in self.faulty_validators() {
            pan[faulty_v] = Observation::Faulty;
        }
        for endorsed_hash in &endorsed {
            if !pan.sees_correct(self, endorsed_hash) {
                pan = pan.merge(self, &self.inclusive_panorama(endorsed_hash));
            }
        }
        pan
    }

    /// Returns panorama of a unit where latest entry of the creator is that unit's hash.
    pub(crate) fn inclusive_panorama(&self, uhash: &C::Hash) -> Panorama<C> {
        let unit = self.unit(&uhash);
        let mut pan = unit.panorama.clone();
        pan[unit.creator] = Observation::Correct(*uhash);
        pan
    }
}

/// Returns the round length, given the round exponent.
pub(crate) fn round_len(round_exp: u8) -> TimeDiff {
    TimeDiff::from(1_u64.checked_shl(round_exp.into()).unwrap_or(u64::MAX))
}

/// Returns the time at which the round with the given timestamp and round exponent began.
///
/// The boundaries of rounds with length `1 << round_exp` are multiples of that length, in
/// milliseconds since the epoch. So the beginning of the current round is the greatest multiple
/// of `1 << round_exp` that is less or equal to `timestamp`.
pub(crate) fn round_id(timestamp: Timestamp, round_exp: u8) -> Timestamp {
    // The greatest multiple less or equal to the timestamp is the timestamp with the last
    // `round_exp` bits set to zero.
    (timestamp >> round_exp) << round_exp
}

/// Returns the base-2 logarithm of `x`, rounded down, i.e. the greatest `i` such that
/// `2.pow(i) <= x`. If `x == 0`, it returns `0`.
fn log2(x: u64) -> u32 {
    // Find the least power of two strictly greater than x and count its trailing zeros.
    // Then subtract 1 to get the zeros of the greatest power of two less or equal than x.
    x.saturating_add(1)
        .checked_next_power_of_two()
        .unwrap_or(0)
        .trailing_zeros()
        .saturating_sub(1)
}

/// Returns a pseudorandom `u64` between `1` and `upper` (inclusive).
fn leader_prng(upper: u64, seed: u64) -> u64 {
    ChaCha8Rng::seed_from_u64(seed)
        .gen_range(0..upper)
        .saturating_add(1)
}
