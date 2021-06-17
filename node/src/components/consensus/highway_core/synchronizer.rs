use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Debug,
    iter,
};

use datasize::DataSize;
use itertools::Itertools;
use tracing::{debug, info};

use crate::{
    components::consensus::{
        consensus_protocol::{ProposedBlock, ProtocolOutcome, ProtocolOutcomes},
        protocols::highway::{HighwayConfig, HighwayMessage, ACTION_ID_VERTEX},
        traits::{Context, NodeIdT},
    },
    types::{TimeDiff, Timestamp},
};

use super::{
    highway::{Dependency, Highway, PreValidatedVertex, ValidVertex, Vertex},
    validators::ValidatorMap,
};

#[cfg(test)]
mod tests;

/// Incoming pre-validated vertices that we haven't added to the protocol state yet, and the
/// timestamp when we received them.
#[derive(DataSize, Debug)]
pub(crate) struct PendingVertices<I, C>(HashMap<PreValidatedVertex<C>, HashMap<I, Timestamp>>)
where
    C: Context;

impl<I, C: Context> Default for PendingVertices<I, C> {
    fn default() -> Self {
        PendingVertices(Default::default())
    }
}

impl<I: NodeIdT, C: Context> PendingVertices<I, C> {
    /// Removes expired vertices.
    fn remove_expired(&mut self, oldest: Timestamp) {
        for time_by_sender in self.0.values_mut() {
            time_by_sender.retain(|_, time_received| *time_received >= oldest);
        }
        self.0.retain(|_, time_by_peer| !time_by_peer.is_empty())
    }

    /// Adds a vertex, or updates its timestamp.
    fn add(&mut self, sender: I, pvv: PreValidatedVertex<C>, time_received: Timestamp) {
        self.0
            .entry(pvv)
            .or_default()
            .entry(sender)
            .and_modify(|timestamp| *timestamp = (*timestamp).max(time_received))
            .or_insert(time_received);
    }

    /// Adds a vertex, or updates its timestamp.
    fn push(&mut self, pv: PendingVertex<I, C>) {
        self.add(pv.sender, pv.pvv, pv.time_received)
    }

    fn pop(&mut self) -> Option<PendingVertex<I, C>> {
        let pvv = self.0.keys().next()?.clone();
        let (sender, timestamp, is_empty) = {
            let time_by_sender = self.0.get_mut(&pvv)?;
            let sender = time_by_sender.keys().next()?.clone();
            let timestamp = time_by_sender.remove(&sender)?;
            (sender, timestamp, time_by_sender.is_empty())
        };
        if is_empty {
            self.0.remove(&pvv);
        }
        Some(PendingVertex::new(sender, pvv, timestamp))
    }

    /// Returns whether dependency exists in the pending vertices collection.
    fn contains_dependency(&self, d: &Dependency<C>) -> bool {
        self.0.keys().any(|pvv| match (&pvv.inner().id(), d) {
            (Dependency::UnitWithPanorama(hash0), Dependency::Unit(hash1)) => hash0 == hash1,
            (dep0, dep1) => dep0 == dep1,
        })
    }

    /// Drops all pending vertices other than evidence.
    pub(crate) fn retain_evidence_only(&mut self) {
        self.0.retain(|pvv, _| pvv.inner().is_evidence());
    }

    /// Returns number of unique vertices pending in the queue.
    pub(crate) fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<I: NodeIdT, C: Context> Iterator for PendingVertices<I, C> {
    type Item = PendingVertex<I, C>;

    fn next(&mut self) -> Option<Self::Item> {
        self.pop()
    }
}

/// An incoming pre-validated vertex that we haven't added to the protocol state yet.
#[derive(DataSize, Debug)]
pub(crate) struct PendingVertex<I, C>
where
    C: Context,
{
    /// The peer who sent it to us.
    sender: I,
    /// The pre-validated vertex.
    pvv: PreValidatedVertex<C>,
    /// The time when we received it.
    time_received: Timestamp,
}

impl<I, C: Context> PendingVertex<I, C> {
    /// Returns a new pending vertex with the current timestamp.
    pub(crate) fn new(sender: I, pvv: PreValidatedVertex<C>, time_received: Timestamp) -> Self {
        Self {
            sender,
            pvv,
            time_received,
        }
    }

    /// Returns the peer from which we received this vertex.
    pub(crate) fn sender(&self) -> &I {
        &self.sender
    }

    /// Returns the vertex waiting to be added.
    pub(crate) fn vertex(&self) -> &Vertex<C> {
        self.pvv.inner()
    }

    /// Returns the pre-validated vertex.
    pub(crate) fn pvv(&self) -> &PreValidatedVertex<C> {
        &self.pvv
    }
}

impl<I, C: Context> Into<PreValidatedVertex<C>> for PendingVertex<I, C> {
    fn into(self) -> PreValidatedVertex<C> {
        self.pvv
    }
}

#[derive(DataSize, Debug)]
pub(crate) struct Synchronizer<I, C>
where
    C: Context,
{
    /// Incoming vertices we can't add yet because they are still missing a dependency.
    vertices_awaiting_deps: BTreeMap<Dependency<C>, PendingVertices<I, C>>,
    /// The vertices that are scheduled to be processed at a later time.  The keys of this
    /// `BTreeMap` are timestamps when the corresponding vector of vertices will be added.
    vertices_to_be_added_later: BTreeMap<Timestamp, PendingVertices<I, C>>,
    /// Vertices that might be ready to add to the protocol state: We are not currently waiting for
    /// a requested dependency.
    vertices_no_deps: PendingVertices<I, C>,
    /// This node's local Highway protocol configuration.
    config: HighwayConfig,
    /// Instance ID of an era for which this synchronizer is constructed.
    instance_id: C::InstanceId,
    /// Keeps track of the lowest/oldest seen unit per validator when syncing.
    /// Used only for logging.
    oldest_seen_panorama: ValidatorMap<Option<u64>>,
    /// Keeps track of the requests we've sent so far and the recipients.
    /// Used to decide whether we should ask more nodes for a particular dependency.
    requests_sent: BTreeMap<Dependency<C>, HashSet<I>>,
    /// Boolean flag indicating whether we're synchronizing current era.
    pub(crate) current_era: bool,
}

impl<I: NodeIdT, C: Context + 'static> Synchronizer<I, C> {
    /// Creates a new synchronizer with the specified timeout for pending vertices.
    pub(crate) fn new(
        config: HighwayConfig,
        validator_len: usize,
        instance_id: C::InstanceId,
    ) -> Self {
        Synchronizer {
            vertices_awaiting_deps: BTreeMap::new(),
            vertices_to_be_added_later: BTreeMap::new(),
            vertices_no_deps: Default::default(),
            config,
            oldest_seen_panorama: iter::repeat(None).take(validator_len).collect(),
            instance_id,
            requests_sent: BTreeMap::new(),
            current_era: true,
        }
    }

    /// Removes expired pending vertices from the queues, and schedules the next purge.
    pub(crate) fn purge_vertices(&mut self, now: Timestamp) {
        info!("purging synchronizer queues");
        let oldest = now.saturating_sub(self.config.pending_vertex_timeout);
        self.vertices_no_deps.remove_expired(oldest);
        self.requests_sent.clear();
        Self::remove_expired(&mut self.vertices_to_be_added_later, oldest);
        Self::remove_expired(&mut self.vertices_awaiting_deps, oldest);
    }

    // Returns number of elements in the `vertices_to_be_added_later` queue.
    // Every pending vertex is counted once, even if it has multiple senders.
    fn vertices_to_be_added_later_len(&self) -> u64 {
        self.vertices_to_be_added_later
            .iter()
            .map(|(_, pv)| pv.len())
            .sum()
    }

    // Returns number of elements in `vertex_deps` queue.
    fn vertices_awaiting_deps_len(&self) -> u64 {
        self.vertices_awaiting_deps
            .iter()
            .map(|(_, pv)| pv.len())
            .sum()
    }

    // Returns number of elements in `vertices_to_be_added` queue.
    fn vertices_no_deps_len(&self) -> u64 {
        self.vertices_no_deps.len()
    }

    pub(crate) fn log_len(&self) {
        debug!(
            era_id = ?self.instance_id,
            vertices_to_be_added_later = self.vertices_to_be_added_later_len(),
            vertices_no_deps = self.vertices_no_deps_len(),
            vertices_awaiting_deps = self.vertices_awaiting_deps_len(),
            "synchronizer queue lengths"
        );
        // All units seen have seq_number == 0.
        let all_lowest = self
            .oldest_seen_panorama
            .iter()
            .all(|entry| entry.map(|seq_num| seq_num == 0).unwrap_or(false));
        if all_lowest {
            debug!("all seen units while synchronization with seq_num=0");
        } else {
            debug!(oldest_panorama=%self.oldest_seen_panorama, "oldest seen unit per validator");
        }
    }

    /// Store a (pre-validated) vertex which will be added later.  This creates a timer to be sent
    /// to the reactor. The vertex be added using `Self::add_vertices` when that timer goes off.
    pub(crate) fn store_vertex_for_addition_later(
        &mut self,
        future_timestamp: Timestamp,
        now: Timestamp,
        sender: I,
        pvv: PreValidatedVertex<C>,
    ) {
        self.vertices_to_be_added_later
            .entry(future_timestamp)
            .or_default()
            .add(sender, pvv, now);
    }

    /// Schedules calls to `add_vertex` on any vertices in `vertices_to_be_added_later` which are
    /// scheduled for after the given `transpired_timestamp`.  In general the specified `timestamp`
    /// is approximately `Timestamp::now()`.  Vertices keyed by timestamps chronologically before
    /// `transpired_timestamp` should all be added.
    pub(crate) fn add_past_due_stored_vertices(
        &mut self,
        timestamp: Timestamp,
    ) -> ProtocolOutcomes<I, C> {
        let mut results = vec![];
        let past_due_timestamps: Vec<Timestamp> = self
            .vertices_to_be_added_later
            .range(..=timestamp) // Inclusive range
            .map(|(past_due_timestamp, _)| past_due_timestamp.to_owned())
            .collect();
        for past_due_timestamp in past_due_timestamps {
            if let Some(vertices_to_add) =
                self.vertices_to_be_added_later.remove(&past_due_timestamp)
            {
                results.extend(self.schedule_add_vertices(vertices_to_add))
            }
        }
        results
    }

    /// Schedules a vertex to be added to the protocol state.
    pub(crate) fn schedule_add_vertex(
        &mut self,
        sender: I,
        pvv: PreValidatedVertex<C>,
        now: Timestamp,
    ) -> ProtocolOutcomes<I, C> {
        self.update_last_seen(&pvv);
        let pv = PendingVertex::new(sender, pvv, now);
        self.schedule_add_vertices(iter::once(pv))
    }

    fn update_last_seen(&mut self, pvv: &PreValidatedVertex<C>) {
        let v = pvv.inner();
        if let (Some(v_id), Some(seq_num)) = (v.creator(), v.unit_seq_number()) {
            let prev_seq_num = self.oldest_seen_panorama[v_id].unwrap_or(u64::MAX);
            self.oldest_seen_panorama[v_id] = Some(prev_seq_num.min(seq_num));
        }
    }

    /// Moves all vertices whose known missing dependency is now satisfied into the
    /// `vertices_to_be_added` queue.
    pub(crate) fn remove_satisfied_deps(&mut self, highway: &Highway<C>) -> ProtocolOutcomes<I, C> {
        let satisfied_deps = self
            .vertices_awaiting_deps
            .keys()
            .filter(|dep| highway.has_dependency(dep) != Some(false))
            .cloned()
            .collect_vec();
        // Safe to unwrap: We know the keys exist. TODO: Replace with BTreeMap::retain once stable.
        let pvs = satisfied_deps
            .into_iter()
            .flat_map(|dep| {
                self.requests_sent.remove(&dep);
                self.vertices_awaiting_deps.remove(&dep).unwrap()
            })
            .collect_vec();
        self.schedule_add_vertices(pvs)
    }

    /// Pops and returns the next entry from `vertices_to_be_added` that is not yet in the protocol
    /// state. Also returns a `ProtocolOutcome` that schedules the next action to add a vertex,
    /// unless the queue is empty, and `ProtocolOutcome`s to request missing dependencies.
    pub(crate) fn pop_vertex_to_add(
        &mut self,
        highway: &Highway<C>,
        pending_values: &HashMap<ProposedBlock<C>, HashSet<(ValidVertex<C>, I)>>,
    ) -> (Option<PendingVertex<I, C>>, ProtocolOutcomes<I, C>) {
        let mut outcomes = Vec::new();
        // Get the next vertex to be added; skip the ones that are already in the protocol state,
        // and the ones that are still missing dependencies.
        loop {
            let pv = match self.vertices_no_deps.pop() {
                None => return (None, outcomes),
                Some(pv) if highway.has_vertex(pv.vertex()) => continue,
                Some(pv) => pv,
            };
            if let Some(dep) = highway.missing_dependency(pv.pvv()) {
                // Find the first dependency that `pv` needs that we haven't synchronized yet
                // and request it from the sender of `pv`. Since it relies on it, it should have
                // it as well.
                let transitive_dependency = self.find_transitive_dependency(dep.clone());
                if self
                    .vertices_no_deps
                    .contains_dependency(&transitive_dependency)
                {
                    // `dep` is already downloaded and waiting in the synchronizer queue to be
                    // added, we don't have to request it again. Add the `pv`
                    // back to the queue so that it can be retried later. `dep` does not wait for
                    // any of the dependencies currently so it should be retried soon.
                    self.add_missing_dependency(dep.clone(), pv);
                    continue;
                }
                // We are still missing a dependency. Store the vertex in the map and request
                // the dependency from the sender.
                let sender = pv.sender().clone();
                let time_received = pv.time_received;
                // Make `pv` depend on the direct dependency `dep` and not `transitive_dependency`
                // since there's a higher chance of adding `pv` to the protocol
                // state after `dep` is added, rather than `transitive_dependency`.
                self.add_missing_dependency(dep.clone(), pv);
                // If we already have the dependency and it is a proposal that is currently being
                // handled by the block validator, and this sender is already known as a source,
                // do nothing.
                if pending_values
                    .values()
                    .flatten()
                    .any(|(vv, s)| vv.inner().id().matches(&transitive_dependency) && s == &sender)
                {
                    continue;
                }
                // If we already have the dependency and it is a proposal that is currently being
                // handled by the block validator, and this sender is not yet known as a source,
                // we return the proposal as if this sender had sent it to us, so they get added.
                if let Some((vv, _)) = pending_values
                    .values()
                    .flatten()
                    .find(|(vv, _)| vv.inner().id().matches(&transitive_dependency))
                {
                    info!(
                        dependency = ?transitive_dependency, %sender,
                        "adding sender as a source for proposal"
                    );
                    let dep_pv = PendingVertex::new(sender, vv.clone().into(), time_received);
                    // We found the next vertex to add.
                    if !self.vertices_no_deps.is_empty() {
                        // There are still vertices in the queue: schedule next call.
                        outcomes.push(ProtocolOutcome::QueueAction(ACTION_ID_VERTEX));
                    }
                    return (Some(dep_pv), outcomes);
                }
                // If we have already requested the dependency from this peer, or from the maximum
                // number of peers, do nothing.
                let entry = self
                    .requests_sent
                    .entry(transitive_dependency.clone())
                    .or_default();
                if entry.len() >= self.config.max_requests_for_vertex
                    || !entry.insert(sender.clone())
                {
                    continue;
                }
                // Otherwise request the missing dependency from the sender.
                info!(dependency = ?transitive_dependency, %sender, "requesting dependency");
                let ser_msg = HighwayMessage::RequestDependency(transitive_dependency).serialize();
                outcomes.push(ProtocolOutcome::CreatedTargetedMessage(ser_msg, sender));
                continue;
            }
            // We found the next vertex to add.
            if !self.vertices_no_deps.is_empty() {
                // There are still vertices in the queue: schedule next call.
                outcomes.push(ProtocolOutcome::QueueAction(ACTION_ID_VERTEX));
            }
            return (Some(pv), outcomes);
        }
    }

    // Finds the highest missing dependency (i.e. one that we are waiting to be downloaded) and
    // returns it, if any.
    fn find_transitive_dependency(&self, mut missing_dependency: Dependency<C>) -> Dependency<C> {
        // If `missing_dependency` is already downloaded and waiting for its dependency to be
        // resolved, we will follow that dependency until we find "the bottom" of the
        // chain – when there are no more known dependency requests scheduled,
        // and we request the last one in the chain.
        loop {
            let maybe_next_missing = self
                .vertices_awaiting_deps
                .iter()
                .find(|(_, pv)| pv.contains_dependency(&missing_dependency))
                .map(|(next_dep, _)| next_dep);
            match maybe_next_missing {
                None => {
                    // If we hit the end of a dependency chain, request downloading the last one
                    // missing.
                    break;
                }
                Some(next_missing) => {
                    missing_dependency = next_missing.clone();
                }
            }
        }
        missing_dependency
    }

    /// Adds a vertex with a known missing dependency to the queue.
    fn add_missing_dependency(&mut self, dep: Dependency<C>, pv: PendingVertex<I, C>) {
        self.vertices_awaiting_deps.entry(dep).or_default().push(pv)
    }

    /// Returns `true` if no vertices are in the queues.
    pub(crate) fn is_empty(&self) -> bool {
        self.vertices_awaiting_deps.is_empty()
            && self.vertices_no_deps.is_empty()
            && self.vertices_to_be_added_later.is_empty()
    }

    /// Returns `true` if there are any vertices waiting for the specified dependency.
    pub(crate) fn is_dependency(&self, dep: &Dependency<C>) -> bool {
        self.vertices_awaiting_deps.contains_key(dep)
    }

    /// Returns the timeout for pending vertices: Entries older than this are purged periodically.
    pub(crate) fn pending_vertex_timeout(&self) -> TimeDiff {
        self.config.pending_vertex_timeout
    }

    /// Drops all vertices that (directly or indirectly) have the specified dependencies, and
    /// returns the set of their senders. If the specified dependencies are known to be invalid,
    /// those senders must be faulty.
    pub(crate) fn invalid_vertices(&mut self, mut vertices: Vec<Dependency<C>>) -> HashSet<I> {
        let mut senders = HashSet::new();
        while !vertices.is_empty() {
            let (new_vertices, new_senders) = self.do_drop_dependent_vertices(vertices);
            vertices = new_vertices;
            senders.extend(new_senders);
        }
        senders
    }

    /// Drops all pending vertices other than evidence.
    pub(crate) fn retain_evidence_only(&mut self) {
        self.vertices_awaiting_deps.clear();
        self.vertices_to_be_added_later.clear();
        self.vertices_no_deps.retain_evidence_only();
        self.requests_sent.clear();
    }

    /// Schedules vertices to be added to the protocol state.
    fn schedule_add_vertices<T>(&mut self, pending_vertices: T) -> ProtocolOutcomes<I, C>
    where
        T: IntoIterator<Item = PendingVertex<I, C>>,
    {
        let was_empty = self.vertices_no_deps.is_empty();
        for pv in pending_vertices {
            self.vertices_no_deps.push(pv);
        }
        if was_empty && !self.vertices_no_deps.is_empty() {
            vec![ProtocolOutcome::QueueAction(ACTION_ID_VERTEX)]
        } else {
            Vec::new()
        }
    }

    /// Drops all vertices that have the specified direct dependencies, and returns their IDs and
    /// senders.
    fn do_drop_dependent_vertices(
        &mut self,
        vertices: Vec<Dependency<C>>,
    ) -> (Vec<Dependency<C>>, HashSet<I>) {
        // collect the vertices that depend on the ones we got in the argument and their senders
        vertices
            .into_iter()
            // filtering by is_unit, so that we don't drop vertices depending on invalid evidence
            // or endorsements - we can still get valid ones from someone else and eventually
            // satisfy the dependency
            .filter(|dep| dep.is_unit())
            .flat_map(|vertex| self.vertices_awaiting_deps.remove(&vertex))
            .flatten()
            .map(|pv| (pv.pvv.inner().id(), pv.sender))
            .unzip()
    }

    /// Removes all expired entries from a `BTreeMap` of `Vec`s.
    fn remove_expired<T: Ord + Clone>(
        map: &mut BTreeMap<T, PendingVertices<I, C>>,
        oldest: Timestamp,
    ) {
        for pvs in map.values_mut() {
            pvs.remove_expired(oldest);
        }
        let keys = map
            .iter()
            .filter(|(_, pvs)| pvs.is_empty())
            .map(|(key, _)| key.clone())
            .collect_vec();
        for key in keys {
            map.remove(&key);
        }
    }
}
