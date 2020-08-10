use std::fmt::Debug;

use anyhow::Error;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    components::consensus::{
        consensus_protocol::{
            synchronizer::{DagSynchronizerState, SynchronizerEffect},
            BlockContext, ConsensusProtocol, ConsensusProtocolResult, ProtocolState, VertexTrait,
        },
        highway_core::{
            active_validator::Effect as AvEffect,
            finality_detector::{FinalityDetector, FinalityOutcome},
            highway::{Highway, PreValidatedVertex, ValidVertex},
            validators::Validators,
            vertex::{Dependency, Vertex},
            Weight,
        },
        traits::{Context, NodeIdT, ValidatorSecret},
    },
    crypto::{
        asymmetric_key::{sign, verify, PublicKey, SecretKey, Signature},
        hash::{hash, Digest},
    },
    types::{ProtoBlock, Timestamp},
};

impl<C: Context> VertexTrait for PreValidatedVertex<C> {
    type Id = Dependency<C>;
    type Value = C::ConsensusValue;

    fn id(&self) -> Dependency<C> {
        self.vertex().id()
    }

    fn value(&self) -> Option<&C::ConsensusValue> {
        self.vertex().value()
    }
}

impl<C: Context> ProtocolState for Highway<C> {
    type Error = String;
    type VId = Dependency<C>;
    type Vertex = PreValidatedVertex<C>;

    fn missing_dependency(&self, pvv: &Self::Vertex) -> Option<Dependency<C>> {
        self.missing_dependency(pvv)
    }

    fn get_vertex(&self, v: Dependency<C>) -> Result<Option<PreValidatedVertex<C>>, Self::Error> {
        Ok(self.get_dependency(&v).map(PreValidatedVertex::from))
    }
}

#[derive(Debug)]
pub(crate) struct HighwayProtocol<I, C: Context> {
    synchronizer: DagSynchronizerState<I, Highway<C>>,
    finality_detector: FinalityDetector<C>,
    highway: Highway<C>,
}

impl<I: NodeIdT, C: Context> HighwayProtocol<I, C> {
    #[allow(clippy::too_many_arguments)] // TODO: Those _are_ too many arguments!
    pub(crate) fn new(
        instance_id: C::InstanceId,
        validators: Validators<C::ValidatorId>,
        seed: u64,
        our_id: C::ValidatorId,
        secret: C::ValidatorSecret,
        min_round_exp: u8,
        ftt: Weight,
        timestamp: Timestamp,
    ) -> (Self, Vec<CpResult<I, C>>) {
        // TODO: Get forgiveness factor from the chain spec.
        let mut highway = Highway::new(instance_id, validators, seed, (1, 5), min_round_exp);
        // TODO: We use the minimum as round exponent here, since it is meant to be optimal.
        // For adaptive round lengths we will probably want to use the most recent one from the
        // previous era instead.
        let av_effects = highway.activate_validator(our_id, secret, min_round_exp, timestamp);
        let mut instance = HighwayProtocol {
            synchronizer: DagSynchronizerState::new(),
            finality_detector: FinalityDetector::new(ftt),
            highway,
        };
        let effects = instance.process_av_effects(av_effects);
        (instance, effects)
    }

    fn process_av_effects<E: IntoIterator<Item = AvEffect<C>>>(
        &mut self,
        av_effects: E,
    ) -> Vec<CpResult<I, C>> {
        av_effects
            .into_iter()
            .flat_map(|effect| self.process_av_effect(effect))
            .collect()
    }

    fn process_av_effect(&mut self, effect: AvEffect<C>) -> Vec<CpResult<I, C>> {
        match effect {
            AvEffect::NewVertex(vv) => self.process_new_vertex(vv),
            AvEffect::ScheduleTimer(timestamp) => {
                vec![ConsensusProtocolResult::ScheduleTimer(timestamp)]
            }
            AvEffect::RequestNewBlock(block_context, _opt_parent) => {
                vec![ConsensusProtocolResult::CreateNewBlock(block_context)]
            }
        }
    }

    fn process_new_vertex(&mut self, vv: ValidVertex<C>) -> Vec<CpResult<I, C>> {
        let msg = HighwayMessage::NewVertex(vv.clone().into());
        //TODO: Don't unwrap
        // Replace serde with generic serializer.
        let serialized_msg = serde_json::to_vec_pretty(&msg).unwrap();
        assert!(
            self.highway.add_valid_vertex(vv).is_empty(),
            "unexpected effects when adding our own vertex"
        );
        let mut results = vec![ConsensusProtocolResult::CreatedGossipMessage(
            serialized_msg,
        )];
        match self.finality_detector.run(&self.highway) {
            FinalityOutcome::None => (),
            FinalityOutcome::FttExceeded => panic!("Too many faulty validators"),
            FinalityOutcome::Finalized {
                value,
                new_equivocators,
                rewards,
                timestamp,
            } => {
                results.push(ConsensusProtocolResult::FinalizedBlock {
                    value,
                    new_equivocators,
                    rewards,
                    timestamp,
                });
            }
        }
        results
    }
}

#[derive(Serialize, Deserialize)]
#[serde(bound(
    serialize = "C::Hash: Serialize",
    deserialize = "C::Hash: Deserialize<'de>",
))]
enum HighwayMessage<C: Context> {
    NewVertex(Vertex<C>),
    RequestDependency(Dependency<C>),
}

type CpResult<I, C> =
    ConsensusProtocolResult<I, <C as Context>::ConsensusValue, <C as Context>::ValidatorId>;

struct SynchronizerQueue<'a, I, C: Context> {
    vertex_queue: Vec<(I, PreValidatedVertex<C>)>,
    synchronizer_effects_queue: Vec<SynchronizerEffect<I, PreValidatedVertex<C>>>,
    results: Vec<CpResult<I, C>>,
    hw_proto: &'a mut HighwayProtocol<I, C>,
}

impl<'a, I, C: Context> SynchronizerQueue<'a, I, C>
where
    I: NodeIdT,
{
    fn new(hw_proto: &'a mut HighwayProtocol<I, C>) -> Self {
        Self {
            vertex_queue: vec![],
            synchronizer_effects_queue: vec![],
            results: vec![],
            hw_proto,
        }
    }

    fn with_vertices(mut self, vertices: Vec<(I, PreValidatedVertex<C>)>) -> Self {
        self.vertex_queue = vertices;
        self
    }

    fn with_synchronizer_effects(
        mut self,
        effects: Vec<SynchronizerEffect<I, PreValidatedVertex<C>>>,
    ) -> Self {
        self.synchronizer_effects_queue = effects;
        self
    }

    fn run(mut self) -> Vec<CpResult<I, C>> {
        loop {
            if let Some(effect) = self.synchronizer_effects_queue.pop() {
                self.process_synchronizer_effect(effect);
            } else if let Some((sender, vertex)) = self.vertex_queue.pop() {
                self.process_vertex(sender, vertex);
            } else {
                return self.results;
            }
        }
    }

    fn process_vertex(&mut self, sender: I, vertex: PreValidatedVertex<C>) {
        match self
            .hw_proto
            .synchronizer
            .synchronize_vertex(sender, vertex, &self.hw_proto.highway)
        {
            Ok(effects) => self.synchronizer_effects_queue.extend(effects),
            Err(err) => todo!("error: {:?}", err),
        }
    }

    fn process_synchronizer_effect(
        &mut self,
        effect: SynchronizerEffect<I, PreValidatedVertex<C>>,
    ) {
        match effect {
            SynchronizerEffect::RequestVertex(sender, missing_vid) => {
                let msg = HighwayMessage::RequestDependency(missing_vid);
                let serialized_msg = match serde_json::to_vec_pretty(&msg) {
                    Ok(msg) => msg,
                    Err(err) => todo!("error: {:?}", err),
                };
                self.results
                    .push(ConsensusProtocolResult::CreatedTargetedMessage(
                        serialized_msg,
                        sender,
                    ));
            }
            SynchronizerEffect::Ready(pvv) => {
                let vv = match self.hw_proto.highway.validate_vertex(pvv) {
                    Ok(vv) => vv,
                    Err((pvv, err)) => {
                        // TODO: Disconnect from sender!
                        // TODO: Remove all vertices from the synchronizer that depend on this one.
                        info!(?pvv, ?err, "invalid vertex");
                        return;
                    }
                };
                // TODO: Avoid cloning. (Serialize first?)
                let av_effects = self.hw_proto.highway.add_valid_vertex(vv.clone());
                self.results
                    .extend(self.hw_proto.process_av_effects(av_effects));
                let msg = HighwayMessage::NewVertex(vv.into());
                // TODO: Don't `unwrap`.
                let serialized_msg = serde_json::to_vec_pretty(&msg).unwrap();
                self.results
                    .push(ConsensusProtocolResult::CreatedGossipMessage(
                        serialized_msg,
                    ))
            }
            SynchronizerEffect::RequeueVertex(sender, vertex) => {
                self.vertex_queue.push((sender, vertex));
            }
            SynchronizerEffect::RequestConsensusValue(sender, value) => {
                self.results
                    .push(ConsensusProtocolResult::ValidateConsensusValue(
                        sender, value,
                    ));
            }
        }
    }
}

impl<I, C: Context> ConsensusProtocol<I, C::ConsensusValue, C::ValidatorId>
    for HighwayProtocol<I, C>
where
    I: NodeIdT,
{
    fn handle_message(&mut self, sender: I, msg: Vec<u8>) -> Result<Vec<CpResult<I, C>>, Error> {
        let highway_message: HighwayMessage<C> = serde_json::from_slice(msg.as_slice()).unwrap();
        Ok(match highway_message {
            HighwayMessage::NewVertex(ref v) if self.highway.has_vertex(v) => vec![],
            HighwayMessage::NewVertex(v) => {
                let pvv = match self.highway.pre_validate_vertex(v) {
                    Ok(pvv) => pvv,
                    Err((_vertex, err)) => {
                        return Ok(vec![ConsensusProtocolResult::InvalidIncomingMessage(
                            msg,
                            sender,
                            err.into(),
                        )]);
                    }
                };
                // TODO: Is there a danger that this takes too much time, and starves other
                // components and events? Consider replacing the loop with a "callback" effect:
                // Instead of handling `HighwayMessage::NewVertex(v)` directly, return a
                // `EnqueueVertex(v)` that causes the reactor to call us with an
                // `Event::NewVertex(v)`, and call `add_vertex` when handling that event. For each
                // returned vertex that needs to be requeued, also return an `EnqueueVertex`
                // effect.
                SynchronizerQueue::new(self)
                    .with_vertices(vec![(sender, pvv)])
                    .run()
            }
            HighwayMessage::RequestDependency(dep) => {
                if let Some(vv) = self.highway.get_dependency(&dep) {
                    let msg = HighwayMessage::NewVertex(vv.into());
                    let serialized_msg = serde_json::to_vec_pretty(&msg).unwrap();
                    // TODO: Should this be done via a gossip service?
                    vec![ConsensusProtocolResult::CreatedTargetedMessage(
                        serialized_msg,
                        sender,
                    )]
                } else {
                    info!(?dep, "Requested dependency doesn't exist.");
                    vec![]
                }
            }
        })
    }

    fn handle_timer(&mut self, timestamp: Timestamp) -> Result<Vec<CpResult<I, C>>, Error> {
        let effects = self.highway.handle_timer(timestamp);
        Ok(self.process_av_effects(effects))
    }

    fn propose(
        &mut self,
        value: C::ConsensusValue,
        block_context: BlockContext,
    ) -> Result<Vec<CpResult<I, C>>, Error> {
        let effects = self.highway.propose(value, block_context);
        Ok(self.process_av_effects(effects))
    }

    /// Marks `value` as valid.
    /// Calls the synchronizer that `value` dependency has been satisfied.
    fn resolve_validity(
        &mut self,
        value: &C::ConsensusValue,
        valid: bool,
    ) -> Result<Vec<CpResult<I, C>>, Error> {
        if valid {
            let effects = self.synchronizer.on_consensus_value_synced(value);
            Ok(SynchronizerQueue::new(self)
                .with_synchronizer_effects(effects)
                .run())
        } else {
            todo!("Drop vertices that depend on the invalid consensus value.")
        }
    }

    /// Turns this instance into a passive observer, that does not create any new vertices.
    fn deactivate_validator(&mut self) {
        self.highway.deactivate_validator()
    }
}

pub(crate) struct HighwaySecret {
    secret_key: SecretKey,
    public_key: PublicKey,
}

impl HighwaySecret {
    pub(crate) fn new(secret_key: SecretKey, public_key: PublicKey) -> Self {
        Self {
            secret_key,
            public_key,
        }
    }
}

impl ValidatorSecret for HighwaySecret {
    type Hash = Digest;
    type Signature = Signature;

    fn sign(&self, data: &Digest) -> Signature {
        sign(data, &self.secret_key, &self.public_key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct HighwayContext;

impl Context for HighwayContext {
    type ConsensusValue = ProtoBlock;
    type ValidatorId = PublicKey;
    type ValidatorSecret = HighwaySecret;
    type Signature = Signature;
    type Hash = Digest;
    type InstanceId = Digest;

    fn hash(data: &[u8]) -> Digest {
        hash(data)
    }

    fn verify_signature(hash: &Digest, public_key: &PublicKey, signature: &Signature) -> bool {
        verify(hash, signature, public_key).is_ok()
    }
}
