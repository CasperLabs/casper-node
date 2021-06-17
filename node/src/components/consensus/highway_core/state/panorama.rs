use std::{collections::HashSet, fmt::Debug};

use datasize::DataSize;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{
    components::consensus::{
        highway_core::{
            highway::{Dependency, ObservationSeqNum},
            state::{State, Unit, UnitError},
            validators::{ValidatorIndex, ValidatorMap},
        },
        traits::Context,
    },
    types::Timestamp,
};

/// The observed behavior of a validator at some point in time.
#[derive(Clone, DataSize, Eq, PartialEq, Serialize, Deserialize, Hash)]
#[serde(bound(
    serialize = "C::Hash: Serialize",
    deserialize = "C::Hash: Deserialize<'de>",
))]
pub(crate) enum Observation<C>
where
    C: Context,
{
    /// No unit by that validator was observed yet.
    None,
    /// The validator's latest unit.
    Correct(C::Hash),
    /// The validator has been seen
    Faulty,
}

impl<C: Context> Debug for Observation<C>
where
    C::Hash: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Observation::None => write!(f, "N"),
            Observation::Faulty => write!(f, "F"),
            Observation::Correct(hash) => write!(f, "{:?}", hash),
        }
    }
}

impl<C: Context> Observation<C> {
    /// Returns the unit hash, if this is a correct observation.
    pub(crate) fn correct(&self) -> Option<&C::Hash> {
        match self {
            Self::None | Self::Faulty => None,
            Self::Correct(hash) => Some(hash),
        }
    }

    pub(crate) fn is_correct(&self) -> bool {
        match self {
            Self::None | Self::Faulty => false,
            Self::Correct(_) => true,
        }
    }

    pub(crate) fn is_faulty(&self) -> bool {
        match self {
            Self::Faulty => true,
            Self::None | Self::Correct(_) => false,
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        match self {
            Self::None => true,
            Self::Faulty | Self::Correct(_) => false,
        }
    }

    /// Returns whether `self` can come later in time than `other`.
    fn geq(&self, state: &State<C>, other: &Observation<C>) -> bool {
        match (self, other) {
            (Observation::Faulty, _) | (_, Observation::None) => true,
            (Observation::Correct(hash0), Observation::Correct(hash1)) => {
                hash0 == hash1 || state.unit(hash0).panorama.sees_correct(state, hash1)
            }
            (_, _) => false,
        }
    }

    /// Returns the missing dependency if `self` is referring to a vertex we don't know yet.
    fn missing_dep(&self, state: &State<C>, idx: ValidatorIndex) -> Option<Dependency<C>> {
        match self {
            Observation::Faulty if !state.is_faulty(idx) => Some(Dependency::Evidence(idx)),
            Observation::Correct(hash) if !state.has_unit(hash) => Some(Dependency::Unit(*hash)),
            _ => None,
        }
    }
}

/// The observed behavior of all validators at some point in time.
pub(crate) type Panorama<C> = ValidatorMap<Observation<C>>;

impl<C: Context> Panorama<C> {
    /// Creates a new, empty panorama.
    pub(crate) fn new(num_validators: usize) -> Panorama<C> {
        Panorama::from(vec![Observation::None; num_validators])
    }

    /// Returns `true` if there is at least one correct observation.
    pub(crate) fn has_correct(&self) -> bool {
        self.iter().any(Observation::is_correct)
    }

    /// Returns an iterator over all honest validators' latest units.
    pub(crate) fn iter_correct<'a>(
        &'a self,
        state: &'a State<C>,
    ) -> impl Iterator<Item = &'a Unit<C>> {
        let to_unit = move |vh: &C::Hash| state.unit(vh);
        self.iter_correct_hashes().map(to_unit)
    }

    /// Returns an iterator over all honest validators' latest units' hashes.
    pub(crate) fn iter_correct_hashes(&self) -> impl Iterator<Item = &C::Hash> {
        self.iter().filter_map(Observation::correct)
    }

    /// Returns an iterator over all faulty validators' indices.
    pub(crate) fn iter_faulty(&self) -> impl Iterator<Item = ValidatorIndex> + '_ {
        self.enumerate()
            .filter(|(_, obs)| obs.is_faulty())
            .map(|(i, _)| i)
    }

    /// Returns an iterator over all faulty validators' indices.
    pub(crate) fn iter_none(&self) -> impl Iterator<Item = ValidatorIndex> + '_ {
        self.enumerate()
            .filter(|(_, obs)| obs.is_none())
            .map(|(i, _)| i)
    }

    /// Returns the correct sequence number for a new unit by `vidx` with this panorama.
    pub(crate) fn next_seq_num(&self, state: &State<C>, vidx: ValidatorIndex) -> u64 {
        // In a trillion years, we need to make seq number u128.
        #[allow(clippy::integer_arithmetic)]
        let add1 = |vh: &C::Hash| state.unit(vh).seq_number + 1;
        self[vidx].correct().map_or(0, add1)
    }

    /// Returns `true` if `self` sees the creator of `hash` as correct, and sees that unit.
    pub(crate) fn sees_correct(&self, state: &State<C>, hash: &C::Hash) -> bool {
        let unit = state.unit(hash);
        let can_see = |latest_hash: &C::Hash| {
            Some(hash) == state.find_in_swimlane(latest_hash, unit.seq_number)
        };
        self.get(unit.creator).correct().map_or(false, can_see)
    }

    /// Returns `true` if `self` sees the unit with the specified `hash`.
    pub(crate) fn sees(&self, state: &State<C>, hash_to_be_found: &C::Hash) -> bool {
        let unit_to_be_found = state.unit(hash_to_be_found);
        let mut visited = HashSet::new();
        let mut to_visit: Vec<_> = self.iter_correct_hashes().collect();
        while let Some(hash) = to_visit.pop() {
            if visited.insert(hash) {
                if hash == hash_to_be_found {
                    return true;
                }
                let unit = state.unit(hash);
                // If the creator is seen as faulty, we need to continue traversing the whole DAG.
                // If it is correct, we only need to follow their own units.
                match &unit.panorama[unit_to_be_found.creator] {
                    Observation::Faulty => to_visit.extend(unit.panorama.iter_correct_hashes()),
                    Observation::Correct(prev_hash) => to_visit.push(prev_hash),
                    Observation::None => (),
                }
            }
        }
        false
    }

    /// Merges two panoramas into a new one.
    pub(crate) fn merge(&self, state: &State<C>, other: &Panorama<C>) -> Panorama<C> {
        let merge_obs = |observations: (&Observation<C>, &Observation<C>)| match observations {
            (Observation::Faulty, _) | (_, Observation::Faulty) => Observation::Faulty,
            (Observation::None, obs) | (obs, Observation::None) => obs.clone(),
            (obs0, Observation::Correct(vh1)) if self.sees_correct(state, vh1) => obs0.clone(),
            (Observation::Correct(vh0), obs1) if other.sees_correct(state, vh0) => obs1.clone(),
            (Observation::Correct(_), Observation::Correct(_)) => Observation::Faulty,
        };
        let observations = self.iter().zip(other).map(merge_obs).collect_vec();
        Panorama::from(observations)
    }

    /// Returns the panorama seeing all units seen by `self` with a timestamp no later than
    /// `timestamp`. Accusations are preserved regardless of the evidence's timestamp.
    pub(crate) fn cutoff(&self, state: &State<C>, timestamp: Timestamp) -> Panorama<C> {
        let obs_cutoff = |obs: &Observation<C>| match obs {
            Observation::Correct(vhash) => state
                .swimlane(vhash)
                .find(|(_, unit)| unit.timestamp <= timestamp)
                .map(|(vh, _)| *vh)
                .map_or(Observation::None, Observation::Correct),
            obs @ Observation::None | obs @ Observation::Faulty => obs.clone(),
        };
        Panorama::from(self.iter().map(obs_cutoff).collect_vec())
    }

    /// Returns the first missing dependency, or `None` if all are satisfied.
    pub(crate) fn missing_dependency(&self, state: &State<C>) -> Option<Dependency<C>> {
        let missing_dep = |(idx, obs): (_, &Observation<C>)| obs.missing_dep(state, idx);
        self.enumerate().filter_map(missing_dep).next()
    }

    /// Returns the checksum of all hashes in this panorama.
    pub(crate) fn hash(&self) -> C::Hash {
        <C as Context>::hash(&bincode::serialize(self).expect("serialize Panorama"))
    }

    /// Returns the sequence number-based version of this panorama.
    ///
    /// Panics if the state doesn't have all cited units.
    pub(crate) fn to_seq_num_panorama(&self, state: &State<C>) -> ValidatorMap<ObservationSeqNum> {
        self.iter()
            .map(|obs| match obs {
                Observation::None => ObservationSeqNum::None,
                Observation::Faulty => ObservationSeqNum::Faulty,
                Observation::Correct(hash) => {
                    ObservationSeqNum::Correct(state.unit(hash).seq_number)
                }
            })
            .collect()
    }

    /// Returns whether `self` can possibly come later in time than `other`, i.e. it can see
    /// every honest message and every fault seen by `other`.
    pub(super) fn geq(&self, state: &State<C>, other: &Panorama<C>) -> bool {
        let mut pairs_iter = self.iter().zip(other);
        pairs_iter.all(|(obs_self, obs_other)| obs_self.geq(state, obs_other))
    }

    /// Returns `Ok(())` if `self` is valid, i.e. it contains the latest units of some substate.
    ///
    /// Panics if the unit has missing dependencies.
    pub(super) fn validate(&self, state: &State<C>) -> Result<(), UnitError> {
        for (idx, observation) in self.enumerate() {
            if let Some(hash) = observation.correct() {
                let unit = state.unit(hash);
                if unit.creator != idx {
                    return Err(UnitError::PanoramaIndex(unit.creator, idx));
                }
                if !self.geq(state, &unit.panorama) {
                    return Err(UnitError::InconsistentPanorama(idx));
                }
            }
        }
        Ok(())
    }
}
