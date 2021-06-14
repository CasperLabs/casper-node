//! A network message type used for communication between nodes

use std::fmt::{self, Display, Formatter};

use derive_more::From;
use fmt::Debug;
use serde::{Deserialize, Serialize};

use crate::{
    components::{
        consensus, gossiper,
        small_network::{GossipedAddress, MessageKind, Payload},
    },
    types::{Deploy, FinalitySignature, Item, Tag},
};
use casper_types::check_summed_hex;

/// Reactor message.
#[derive(Clone, From, Serialize, Deserialize)]
pub enum Message {
    /// Consensus component message.
    #[from]
    Consensus(consensus::ConsensusMessage),
    /// Deploy gossiper component message.
    #[from]
    DeployGossiper(gossiper::Message<Deploy>),
    /// Address gossiper component message.
    #[from]
    AddressGossiper(gossiper::Message<GossipedAddress>),
    /// Request to get an item from a peer.
    GetRequest {
        /// The type tag of the requested item.
        tag: Tag,
        /// The serialized ID of the requested item.
        serialized_id: Vec<u8>,
    },
    /// Response to a `GetRequest`.
    GetResponse {
        /// The type tag of the contained item.
        tag: Tag,
        /// The serialized item.
        serialized_item: Vec<u8>,
    },
    /// Finality signature.
    #[from]
    FinalitySignature(Box<FinalitySignature>),
}

impl Payload for Message {
    #[inline]
    fn classify(&self) -> MessageKind {
        match self {
            Message::Consensus(_) => MessageKind::Consensus,
            Message::DeployGossiper(_) => MessageKind::DeployGossip,
            Message::AddressGossiper(_) => MessageKind::AddressGossip,
            Message::GetRequest { tag, .. } | Message::GetResponse { tag, .. } => {
                match tag {
                    Tag::Deploy => MessageKind::DeployTransfer,
                    Tag::Block => MessageKind::BlockTransfer,
                    // This is a weird message, which we should not encounter here?
                    Tag::GossipedAddress => MessageKind::Other,
                    Tag::BlockByHeight => MessageKind::BlockTransfer,
                    Tag::BlockHeaderByHash => MessageKind::BlockTransfer,
                    Tag::BlockHeaderAndFinalitySignaturesByHeight => MessageKind::BlockTransfer,
                }
            }
            Message::FinalitySignature(_) => MessageKind::Consensus,
        }
    }
}

impl Message {
    pub(crate) fn new_get_request<T: Item>(id: &T::Id) -> Result<Self, bincode::Error> {
        Ok(Message::GetRequest {
            tag: T::TAG,
            serialized_id: bincode::serialize(id)?,
        })
    }

    pub(crate) fn new_get_response<T: Item>(item: &T) -> Result<Self, bincode::Error> {
        Ok(Message::GetResponse {
            tag: T::TAG,
            serialized_item: bincode::serialize(item)?,
        })
    }
}

impl Debug for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Message::Consensus(c) => f.debug_tuple("Consensus").field(&c).finish(),
            Message::DeployGossiper(dg) => f.debug_tuple("DeployGossiper").field(&dg).finish(),
            Message::AddressGossiper(ga) => f.debug_tuple("AddressGossiper").field(&ga).finish(),
            Message::GetRequest { tag, serialized_id } => f
                .debug_struct("GetRequest")
                .field("tag", tag)
                .field("serialized_item", &check_summed_hex::encode(serialized_id))
                .finish(),
            Message::GetResponse {
                tag,
                serialized_item,
            } => f
                .debug_struct("GetResponse")
                .field("tag", tag)
                .field(
                    "serialized_item",
                    &check_summed_hex::encode(serialized_item),
                )
                .finish(),
            Message::FinalitySignature(fs) => {
                f.debug_tuple("FinalitySignature").field(&fs).finish()
            }
        }
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Message::Consensus(consensus) => write!(f, "Consensus::{}", consensus),
            Message::DeployGossiper(deploy) => write!(f, "DeployGossiper::{}", deploy),
            Message::AddressGossiper(gossiped_address) => {
                write!(f, "AddressGossiper::({})", gossiped_address)
            }
            Message::GetRequest { tag, serialized_id } => {
                write!(
                    f,
                    "GetRequest({}-{:10})",
                    tag,
                    check_summed_hex::encode(serialized_id)
                )
            }
            Message::GetResponse {
                tag,
                serialized_item,
            } => write!(
                f,
                "GetResponse({}-{:10})",
                tag,
                check_summed_hex::encode(serialized_item)
            ),
            Message::FinalitySignature(fs) => {
                write!(f, "FinalitySignature::({})", fs)
            }
        }
    }
}
