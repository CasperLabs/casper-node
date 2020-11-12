use crate::{
    components::consensus::{
        highway_core::{
            highway::SignedWireUnit,
            state::{self, Panorama, State},
            validators::ValidatorIndex,
        },
        traits::Context,
    },
    types::{TimeDiff, Timestamp},
};

/// A unit sent to or received from the network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Unit<C: Context> {
    /// The list of latest messages and faults observed by the creator of this message.
    pub(crate) panorama: Panorama<C>,
    /// The number of earlier messages by the same creator.
    pub(crate) seq_number: u64,
    /// The validator who created and sent this unit.
    pub(crate) creator: ValidatorIndex,
    /// The block this unit votes for. Either it or its parent must be the fork choice.
    pub(crate) block: C::Hash,
    /// A skip list index of the creator's swimlane, i.e. the previous unit by the same creator.
    ///
    /// For every `p = 1 << i` that divides `seq_number`, this contains an `i`-th entry pointing to
    /// the older unit with `seq_number - p`.
    pub(crate) skip_idx: Vec<C::Hash>,
    /// This unit's timestamp, in milliseconds since the epoch.
    pub(crate) timestamp: Timestamp,
    /// Original signature of the `SignedWireUnit`.
    pub(crate) signature: C::Signature,
    /// The round exponent of the current round, that this message belongs to.
    ///
    /// The current round consists of all timestamps that agree with this one in all but the last
    /// `round_exp` bits.
    pub(crate) round_exp: u8,
}

impl<C: Context> Unit<C> {
    /// Creates a new `Unit` from the `WireUnit`, and returns the value if it contained any.
    /// Values must be stored as a block, with the same hash.
    pub(crate) fn new(
        swunit: SignedWireUnit<C>,
        fork_choice: Option<&C::Hash>,
        state: &State<C>,
    ) -> (Unit<C>, Option<C::ConsensusValue>) {
        let SignedWireUnit {
            wire_unit: wunit,
            signature,
        } = swunit;
        let block = if wunit.value.is_some() {
            wunit.hash() // A unit with a new block votes for itself.
        } else {
            // If the unit didn't introduce a new block, it votes for the fork choice itself.
            // `Highway::add_unit` checks that the panorama is not empty.
            fork_choice
                .cloned()
                .expect("nonempty panorama has nonempty fork choice")
        };
        let mut skip_idx = Vec::new();
        if let Some(hash) = wunit.panorama.get(wunit.creator).correct() {
            skip_idx.push(*hash);
            for i in 0..wunit.seq_number.trailing_zeros() as usize {
                let old_unit = state.unit(&skip_idx[i]);
                skip_idx.push(old_unit.skip_idx[i]);
            }
        }
        let unit = Unit {
            panorama: wunit.panorama,
            seq_number: wunit.seq_number,
            creator: wunit.creator,
            block,
            skip_idx,
            timestamp: wunit.timestamp,
            signature,
            round_exp: wunit.round_exp,
        };
        (unit, wunit.value)
    }

    /// Returns the creator's previous message.
    pub(crate) fn previous(&self) -> Option<&C::Hash> {
        self.skip_idx.first()
    }

    /// Returns the time at which the round containing this unit began.
    pub(crate) fn round_id(&self) -> Timestamp {
        state::round_id(self.timestamp, self.round_exp)
    }

    /// Returns the length of the round containing this unit.
    pub(crate) fn round_len(&self) -> TimeDiff {
        state::round_len(self.round_exp)
    }

    /// Returns whether `unit` cites a new unit from `vidx` in the last panorama.
    /// i.e. whether previous unit from creator of `vhash` cites different unit by `vidx`.
    ///
    /// NOTE: Returns `false` if `vidx` is faulty or hasn't produced any units according to the
    /// creator of `vhash`.
    pub(crate) fn new_hash_obs(&self, state: &State<C>, vidx: ValidatorIndex) -> bool {
        let latest_obs = self.panorama[vidx].correct();
        let penultimate_obs = self
            .previous()
            .and_then(|v| state.unit(v).panorama[vidx].correct());
        match (latest_obs, penultimate_obs) {
            (Some(latest_hash), Some(penultimate_hash)) => latest_hash != penultimate_hash,
            _ => false,
        }
    }
}
