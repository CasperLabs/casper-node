use std::{
    fmt::{self, Debug, Display, Formatter},
    str::FromStr,
};

use datasize::DataSize;
use libp2p::PeerId;
#[cfg(test)]
use multihash::Multihash;
use once_cell::sync::Lazy;
#[cfg(test)]
use rand::{Rng, RngCore};
use serde::{de::Error as SerdeError, Deserialize, Deserializer, Serialize, Serializer};

#[cfg(test)]
use crate::testing::TestRng;
use crate::{rpcs::docs::DocExample, tls::KeyFingerprint};
use casper_types::check_summed_hex;

/// The network identifier for a node.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, DataSize)]
pub enum NodeId {
    Tls(KeyFingerprint),
    #[data_size(skip)]
    P2p(PeerId),
}

impl NodeId {
    /// Generates a random instance using a `TestRng`.
    #[cfg(test)]
    pub(crate) fn random(rng: &mut TestRng) -> Self {
        if rng.gen() {
            Self::random_tls(rng)
        } else {
            Self::random_p2p(rng)
        }
    }

    /// Generates a random Tls instance using a `TestRng`.
    #[cfg(test)]
    pub(crate) fn random_tls(rng: &mut TestRng) -> Self {
        NodeId::Tls(rng.gen())
    }

    /// Generates a random P2p instance using a `TestRng`.
    #[cfg(test)]
    pub(crate) fn random_p2p(rng: &mut TestRng) -> Self {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes[..]);
        let multihash = Multihash::wrap(multihash::Code::Identity.into(), &bytes).unwrap();
        let peer_id = PeerId::from_multihash(multihash).expect("should construct from multihash");
        NodeId::P2p(peer_id)
    }

    /// Returns the raw bytes of the underlying hash of the ID, if there is any.
    #[inline]
    pub fn hash_bytes(&self) -> Option<&[u8]> {
        if let NodeId::Tls(sha256) = self {
            Some(sha256.as_ref())
        } else {
            None
        }
    }
}

/// Used to serialize and deserialize `NodeID` where the (de)serializer isn't a human-readable type.
#[derive(Serialize, Deserialize)]
enum NodeIdAsBytes {
    Tls(KeyFingerprint),
    P2p(Vec<u8>),
}

/// Used to serialize and deserialize `NodeID` where the (de)serializer is a human-readable type.
#[derive(Serialize, Deserialize)]
enum NodeIdAsString {
    Tls(String),
    P2p(String),
}

impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            let helper = match self {
                NodeId::Tls(key_fingerprint) => {
                    NodeIdAsString::Tls(check_summed_hex::encode(key_fingerprint.as_ref()))
                }
                NodeId::P2p(peer_id) => NodeIdAsString::P2p(peer_id.to_base58()),
            };
            return helper.serialize(serializer);
        }

        let helper = match self {
            NodeId::Tls(key_fingerprint) => NodeIdAsBytes::Tls(*key_fingerprint),
            NodeId::P2p(peer_id) => NodeIdAsBytes::P2p(peer_id.to_bytes()),
        };
        helper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let helper = NodeIdAsString::deserialize(deserializer)?;
            match helper {
                NodeIdAsString::Tls(hex_value) => {
                    let bytes = check_summed_hex::decode(&hex_value).map_err(D::Error::custom)?;
                    if bytes.len() != KeyFingerprint::LENGTH {
                        return Err(SerdeError::custom("wrong length"));
                    }
                    let mut array = [0_u8; KeyFingerprint::LENGTH];
                    array.copy_from_slice(bytes.as_slice());
                    return Ok(NodeId::Tls(KeyFingerprint::from(array)));
                }
                NodeIdAsString::P2p(b58_value) => {
                    let peer_id = PeerId::from_str(&b58_value).map_err(D::Error::custom)?;
                    return Ok(NodeId::P2p(peer_id));
                }
            }
        }

        let helper = NodeIdAsBytes::deserialize(deserializer)?;
        match helper {
            NodeIdAsBytes::Tls(key_fingerprint) => Ok(NodeId::Tls(key_fingerprint)),
            NodeIdAsBytes::P2p(bytes) => {
                let peer_id =
                    PeerId::from_bytes(&bytes).map_err(|_| D::Error::custom("invalid PeerId"))?;
                Ok(NodeId::P2p(peer_id))
            }
        }
    }
}

static NODE_ID: Lazy<NodeId> =
    Lazy::new(|| NodeId::Tls(KeyFingerprint::from([1u8; KeyFingerprint::LENGTH])));

impl DocExample for NodeId {
    fn doc_example() -> &'static Self {
        &*NODE_ID
    }
}

impl Debug for NodeId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Tls(key_fingerprint) => write!(
                formatter,
                "NodeId::Tls({})",
                check_summed_hex::encode(key_fingerprint.as_ref())
            ),
            NodeId::P2p(peer_id) => write!(formatter, "PeerId::P2p({})", peer_id.to_base58()),
        }
    }
}

impl Display for NodeId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Tls(key_fingerprint) => write!(
                formatter,
                "NodeId::Tls({:10})",
                check_summed_hex::encode(key_fingerprint.as_ref())
            ),
            NodeId::P2p(peer_id) => {
                let base58_peer_id = peer_id.to_base58();
                write!(
                    formatter,
                    "NodeId::P2p({}..{})",
                    &base58_peer_id[8..12],
                    &base58_peer_id[(base58_peer_id.len() - 4)..]
                )
            }
        }
    }
}

impl From<KeyFingerprint> for NodeId {
    fn from(id: KeyFingerprint) -> Self {
        NodeId::Tls(id)
    }
}

impl From<PeerId> for NodeId {
    fn from(id: PeerId) -> Self {
        NodeId::P2p(id)
    }
}

#[cfg(test)]
impl From<[u8; KeyFingerprint::LENGTH]> for NodeId {
    fn from(raw_bytes: [u8; KeyFingerprint::LENGTH]) -> Self {
        NodeId::Tls(KeyFingerprint::from(raw_bytes))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn serde_roundtrip_tls() {
        let mut rng = crate::new_rng();
        let node_id = NodeId::random_tls(&mut rng);
        let serialized = bincode::serialize(&node_id).unwrap();
        let decoded = bincode::deserialize(&serialized).unwrap();
        assert_eq!(node_id, decoded);
    }

    #[test]
    fn serde_roundtrip_p2p() {
        let mut rng = crate::new_rng();
        let node_id = NodeId::random_p2p(&mut rng);
        let serialized = bincode::serialize(&node_id).unwrap();
        let decoded = bincode::deserialize(&serialized).unwrap();
        assert_eq!(node_id, decoded);
    }

    #[test]
    fn json_roundtrip_tls() {
        let mut rng = crate::new_rng();
        let node_id = NodeId::random_tls(&mut rng);
        let json_string = serde_json::to_string_pretty(&node_id).unwrap();
        let decoded = serde_json::from_str(&json_string).unwrap();
        assert_eq!(node_id, decoded);
    }

    #[test]
    fn json_roundtrip_p2p() {
        let mut rng = crate::new_rng();
        let node_id = NodeId::random_p2p(&mut rng);
        let json_string = serde_json::to_string_pretty(&node_id).unwrap();
        let decoded = serde_json::from_str(&json_string).unwrap();
        assert_eq!(node_id, decoded);
    }
}
