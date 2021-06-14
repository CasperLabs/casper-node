//! The consensus component. Provides distributed consensus among the nodes in the network.

#![warn(clippy::integer_arithmetic)]

mod cl_context;
mod config;
mod consensus_protocol;
mod era_supervisor;
#[macro_use]
mod highway_core;
mod metrics;
mod protocols;
#[cfg(test)]
mod tests;
mod traits;

use std::{
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    fmt::{self, Debug, Display, Formatter},
    sync::Arc,
    time::Duration,
};

use datasize::DataSize;
use derive_more::From;
use serde::{Deserialize, Serialize};
use tracing::error;

use casper_types::{check_summed_hex, EraId, PublicKey, U512};

use crate::{
    components::Component,
    effect::{
        announcements::{BlocklistAnnouncement, ConsensusAnnouncement},
        requests::{
            BlockProposerRequest, BlockValidationRequest, ChainspecLoaderRequest, ConsensusRequest,
            ContractRuntimeRequest, LinearChainRequest, NetworkRequest, StorageRequest,
        },
        EffectBuilder, EffectExt, Effects,
    },
    fatal,
    protocol::Message,
    reactor::ReactorEvent,
    types::{ActivationPoint, BlockHash, BlockHeader, BlockPayload, Timestamp},
    NodeRng,
};

pub(crate) use cl_context::ClContext;
pub use config::Config;
pub(crate) use consensus_protocol::{BlockContext, EraReport, ProposedBlock};
pub(crate) use era_supervisor::EraSupervisor;
pub(crate) use protocols::highway::HighwayProtocol;
use traits::NodeIdT;

#[cfg(test)]
pub(crate) use era_supervisor::oldest_bonded_era;

#[derive(DataSize, Clone, Serialize, Deserialize)]
pub enum ConsensusMessage {
    /// A protocol message, to be handled by the instance in the specified era.
    Protocol { era_id: EraId, payload: Vec<u8> },
    /// A request for evidence against the specified validator, from any era that is still bonded
    /// in `era_id`.
    EvidenceRequest { era_id: EraId, pub_key: PublicKey },
}

/// An ID to distinguish different timers. What they are used for is specific to each consensus
/// protocol implementation.
#[derive(DataSize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerId(pub u8);

/// An ID to distinguish queued actions. What they are used for is specific to each consensus
/// protocol implementation.
#[derive(DataSize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionId(pub u8);

#[derive(DataSize, Debug, From)]
pub struct NewBlockPayload {
    era_id: EraId,
    block_payload: Arc<BlockPayload>,
    block_context: BlockContext<ClContext>,
}

#[derive(DataSize, Debug, From)]
pub struct ResolveValidity<I> {
    era_id: EraId,
    sender: I,
    proposed_block: ProposedBlock<ClContext>,
    valid: bool,
}

/// Consensus component event.
#[derive(DataSize, Debug, From)]
pub enum Event<I> {
    /// An incoming network message.
    MessageReceived { sender: I, msg: ConsensusMessage },
    /// A scheduled event to be handled by a specified era.
    Timer {
        era_id: EraId,
        timestamp: Timestamp,
        timer_id: TimerId,
    },
    /// A queued action to be handled by a specific era.
    Action { era_id: EraId, action_id: ActionId },
    /// We are receiving the data we require to propose a new block.
    NewBlockPayload(NewBlockPayload),
    #[from]
    ConsensusRequest(ConsensusRequest),
    /// A new block has been added to the linear chain.
    BlockAdded(Box<BlockHeader>),
    /// The proto-block has been validated.
    ResolveValidity(ResolveValidity<I>),
    /// Deactivate the era with the given ID, unless the number of faulty validators increases.
    DeactivateEra {
        era_id: EraId,
        faulty_num: usize,
        delay: Duration,
    },
    /// Event raised when a new era should be created: once we get the set of validators, the
    /// booking block hash and the seed from the key block.
    CreateNewEra {
        /// The header of the switch block, i.e. the last block before the new era.
        switch_block_header: Box<BlockHeader>,
        /// `Ok(block_hash)` if the booking block was found, `Err(era_id)` if not
        booking_block_hash: Result<BlockHash, EraId>,
    },
    /// Event raised upon initialization, when a number of eras have to be instantiated at once.
    InitializeEras {
        key_blocks: HashMap<EraId, BlockHeader>,
        booking_blocks: HashMap<EraId, BlockHash>,
        /// This is empty except if the activation era still needs to be instantiated: Its
        /// validator set is read from the global state, not from a key block.
        validators: BTreeMap<PublicKey, U512>,
    },
    /// Got the result of checking for an upgrade activation point.
    GotUpgradeActivationPoint(ActivationPoint),
}

impl Debug for ConsensusMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsensusMessage::Protocol { era_id, payload: _ } => {
                write!(f, "Protocol {{ era_id: {:?}, .. }}", era_id)
            }
            ConsensusMessage::EvidenceRequest { era_id, pub_key } => f
                .debug_struct("EvidenceRequest")
                .field("era_id", era_id)
                .field("pub_key", pub_key)
                .finish(),
        }
    }
}

impl Display for ConsensusMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ConsensusMessage::Protocol { era_id, payload } => {
                write!(
                    f,
                    "protocol message {:10} in {}",
                    check_summed_hex::encode(payload),
                    era_id
                )
            }
            ConsensusMessage::EvidenceRequest { era_id, pub_key } => write!(
                f,
                "request for evidence of fault by {} in {} or earlier",
                pub_key, era_id,
            ),
        }
    }
}

impl<I: Debug> Display for Event<I> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Event::MessageReceived { sender, msg } => write!(f, "msg from {:?}: {}", sender, msg),
            Event::Timer {
                era_id,
                timestamp,
                timer_id,
            } => write!(
                f,
                "timer (ID {}) for {} scheduled for timestamp {}",
                timer_id.0, era_id, timestamp,
            ),
            Event::Action { era_id, action_id } => {
                write!(f, "action (ID {}) for {}", action_id.0, era_id)
            }
            Event::NewBlockPayload(NewBlockPayload {
                era_id,
                block_payload,
                block_context,
            }) => write!(
                f,
                "New proto-block for era {:?}: {:?}, {:?}",
                era_id, block_payload, block_context
            ),
            Event::ConsensusRequest(request) => write!(
                f,
                "A request for consensus component hash been received: {:?}",
                request
            ),
            Event::BlockAdded(block_header) => write!(
                f,
                "A block has been added to the linear chain: {}",
                block_header.hash(),
            ),
            Event::ResolveValidity(ResolveValidity {
                era_id,
                sender,
                proposed_block,
                valid,
            }) => write!(
                f,
                "Proposed block received from {:?} for {} is {}: {:?}",
                sender,
                era_id,
                if *valid { "valid" } else { "invalid" },
                proposed_block,
            ),
            Event::DeactivateEra {
                era_id, faulty_num, ..
            } => write!(
                f,
                "Deactivate old {} unless additional faults are observed; faults so far: {}",
                era_id, faulty_num
            ),
            Event::CreateNewEra {
                booking_block_hash,
                switch_block_header,
            } => write!(
                f,
                "New era should be created; booking block hash: {:?}, switch block: {:?}",
                booking_block_hash, switch_block_header
            ),
            Event::InitializeEras { .. } => write!(f, "Starting eras should be initialized"),
            Event::GotUpgradeActivationPoint(activation_point) => {
                write!(f, "new upgrade activation point: {:?}", activation_point)
            }
        }
    }
}

/// A helper trait whose bounds represent the requirements for a reactor event that `EraSupervisor`
/// can work with.
pub trait ReactorEventT<I>:
    ReactorEvent
    + From<Event<I>>
    + Send
    + From<NetworkRequest<I, Message>>
    + From<BlockProposerRequest>
    + From<ConsensusAnnouncement>
    + From<BlockValidationRequest<I>>
    + From<StorageRequest>
    + From<ContractRuntimeRequest>
    + From<ChainspecLoaderRequest>
    + From<LinearChainRequest<I>>
    + From<BlocklistAnnouncement<I>>
{
}

impl<REv, I> ReactorEventT<I> for REv where
    REv: ReactorEvent
        + From<Event<I>>
        + Send
        + From<NetworkRequest<I, Message>>
        + From<BlockProposerRequest>
        + From<ConsensusAnnouncement>
        + From<BlockValidationRequest<I>>
        + From<StorageRequest>
        + From<ContractRuntimeRequest>
        + From<ChainspecLoaderRequest>
        + From<LinearChainRequest<I>>
        + From<BlocklistAnnouncement<I>>
{
}

impl<I, REv> Component<REv> for EraSupervisor<I>
where
    I: NodeIdT,
    REv: ReactorEventT<I>,
{
    type Event = Event<I>;
    type ConstructionError = Infallible;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        mut rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        let mut handling_es = self.handling_wrapper(effect_builder, &mut rng);
        match event {
            Event::Timer {
                era_id,
                timestamp,
                timer_id,
            } => handling_es.handle_timer(era_id, timestamp, timer_id),
            Event::Action { era_id, action_id } => handling_es.handle_action(era_id, action_id),
            Event::MessageReceived { sender, msg } => handling_es.handle_message(sender, msg),
            Event::NewBlockPayload(new_block_payload) => {
                handling_es.handle_new_block_payload(new_block_payload)
            }
            Event::BlockAdded(block_header) => handling_es.handle_block_added(*block_header),
            Event::ResolveValidity(resolve_validity) => {
                handling_es.resolve_validity(resolve_validity)
            }
            Event::DeactivateEra {
                era_id,
                faulty_num,
                delay,
            } => handling_es.handle_deactivate_era(era_id, faulty_num, delay),
            Event::CreateNewEra {
                switch_block_header,
                booking_block_hash,
            } => {
                let booking_block_hash = match booking_block_hash {
                    Ok(hash) => hash,
                    Err(era_id) => {
                        error!(
                            "could not find the booking block in era {}, for era {}",
                            era_id,
                            switch_block_header.era_id().successor()
                        );
                        return fatal!(
                            handling_es.effect_builder,
                            "couldn't get the booking block hash"
                        )
                        .ignore();
                    }
                };
                handling_es.handle_create_new_era(*switch_block_header, booking_block_hash)
            }
            Event::InitializeEras {
                key_blocks,
                booking_blocks,
                validators,
            } => handling_es.handle_initialize_eras(key_blocks, booking_blocks, validators),
            Event::GotUpgradeActivationPoint(activation_point) => {
                handling_es.got_upgrade_activation_point(activation_point)
            }
            Event::ConsensusRequest(ConsensusRequest::Status(responder)) => {
                handling_es.status(responder)
            }
        }
    }
}
