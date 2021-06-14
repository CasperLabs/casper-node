//! Observability for network serialization/deserialization.
//!
//! This module introduces two IDs: [`ConnectionId`] and [`TraceId`]. The [`ConnectionId`] is a
//! unique ID per established connection that can be independently derive by peers on either of a
//! connection. [`TraceId`] identifies a single message, distinguishing even messages that are sent
//! to the same peer with equal contents.

use std::{
    convert::TryFrom,
    fmt::{self, Display, Formatter},
    pin::Pin,
    sync::{Arc, Weak},
};

use bytes::{Bytes, BytesMut};
use openssl::ssl::SslRef;
use pin_project::pin_project;
#[cfg(test)]
use rand::RngCore;
use static_assertions::const_assert;
use tokio_serde::{Deserializer, Serializer};
use tracing::{error, trace, warn};

use super::{tls::KeyFingerprint, Message, Payload};
#[cfg(test)]
use crate::testing::TestRng;
use crate::{
    components::networking_metrics::NetworkingMetrics,
    crypto::hash::{self, Digest},
    types::NodeId,
    utils,
};
use casper_types::check_summed_hex;

/// Lazily-evaluated network message ID generator.
///
/// Calculates a hash for the wrapped value when `Display::fmt` is called.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct TraceId([u8; 8]);

impl Display for TraceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&check_summed_hex::encode(&self.0))
    }
}

/// A metric-updating serializer/deserializer wrapper for network messages.
///
/// Classifies each message given and updates the `NetworkingMetrics` accordingly. Also emits a
/// TRACE-level message to the `net_out` and `net_in` target with a per-message unique hash when
/// a message is sent or received.
#[pin_project]
#[derive(Debug)]
pub struct CountingFormat<F> {
    /// The actual serializer performing the work.
    #[pin]
    inner: F,
    /// Identifier for the connection.
    connection_id: ConnectionId,
    /// Counter for outgoing messages.
    out_count: u64,
    /// Counter for incoming messages.
    in_count: u64,
    /// Our role in the connection.
    role: Role,
    /// Metrics to update.
    metrics: Weak<NetworkingMetrics>,
}

impl<F> CountingFormat<F> {
    /// Creates a new counting formatter.
    #[inline]
    pub(super) fn new(
        metrics: Weak<NetworkingMetrics>,
        connection_id: ConnectionId,
        role: Role,
        inner: F,
    ) -> Self {
        Self {
            metrics,
            connection_id,
            out_count: 0,
            in_count: 0,
            role,
            inner,
        }
    }
}

impl<F, P> Serializer<Arc<Message<P>>> for CountingFormat<F>
where
    F: Serializer<Arc<Message<P>>>,
    P: Payload,
{
    type Error = F::Error;

    #[inline]
    fn serialize(self: Pin<&mut Self>, item: &Arc<Message<P>>) -> Result<Bytes, Self::Error> {
        let this = self.project();
        let projection: Pin<&mut F> = this.inner;

        let serialized = F::serialize(projection, item)?;
        let msg_size = serialized.len() as u64;
        let msg_kind = item.classify();
        NetworkingMetrics::record_payload_out(this.metrics, msg_kind, msg_size);

        let trace_id = this
            .connection_id
            .create_trace_id(this.role.out_flag(), *this.out_count);
        *this.out_count += 1;

        trace!(target: "net_out",
            msg_id = %trace_id,
            msg_size,
            msg_kind = %msg_kind, "sending");

        Ok(serialized)
    }
}

impl<F, P> Deserializer<Message<P>> for CountingFormat<F>
where
    F: Deserializer<Message<P>>,
    P: Payload,
{
    type Error = F::Error;

    #[inline]
    fn deserialize(self: Pin<&mut Self>, src: &BytesMut) -> Result<Message<P>, Self::Error> {
        let this = self.project();
        let projection: Pin<&mut F> = this.inner;

        let msg_size = src.len() as u64;

        let deserialized = F::deserialize(projection, src)?;
        let msg_kind = deserialized.classify();

        let trace_id = this
            .connection_id
            .create_trace_id(this.role.in_flag(), *this.in_count);
        *this.in_count += 1;

        trace!(target: "net_in",
            msg_id = %trace_id,
            msg_size,
            msg_kind = %msg_kind, "received");

        Ok(deserialized)
    }
}

/// An ID identifying a connection.
///
/// The ID is guaranteed to be the same on both ends of the connection, but not guaranteed to be
/// unique or sufficiently random. Do not use it for any cryptographic/security related purposes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) struct ConnectionId([u8; Digest::LENGTH]);

// Invariant assumed by `ConnectionId`, `Digest` must be <= than `KeyFingerprint`.
const_assert!(KeyFingerprint::LENGTH >= Digest::LENGTH);
// We also assume it is at least 12 bytes.
const_assert!(Digest::LENGTH >= 12);

/// Random data derived from TLS connections.
#[derive(Copy, Clone, Debug)]
pub(super) struct TlsRandomData {
    /// Random data extract from the client of the connection.
    combined_random: [u8; 12],
}

/// Zero-randomness.
///
/// Used to check random data.
const ZERO_RANDOMNESS: [u8; 12] = [0; 12];

impl TlsRandomData {
    /// Collects random data from an existing SSL collection.
    ///
    /// Ideally we would use the TLS session ID, but it is not available on outgoing connections at
    /// the times we need it. Instead, we use the `server_random` and `client_random` nonces, which
    /// will be the same on both ends of the connection.
    fn collect(ssl: &SslRef) -> Self {
        // We are using only the first 12 bytes of these 32 byte values here, just in case we missed
        // something in our assessment that hashing these should be safe. Additionally, these values
        // are XOR'd, not concatenated. All this is done to prevent leaking information about these
        // numbers.
        //
        // Some SSL implementations use timestamps for the first four bytes, so to be sufficiently
        // random, we use 4 + 8 bytes of the nonces.
        let mut server_random = [0; 12];
        let mut client_random = [0; 12];

        ssl.server_random(&mut server_random);

        if server_random == ZERO_RANDOMNESS {
            warn!("TLS server random is all zeros");
        }

        ssl.client_random(&mut client_random);

        if server_random == ZERO_RANDOMNESS {
            warn!("TLS client random is all zeros");
        }

        // Combine using XOR.
        utils::xor(&mut server_random, &client_random);

        Self {
            combined_random: server_random,
        }
    }

    /// Creates random `TlsRandomData`.
    #[cfg(test)]
    fn random(rng: &mut TestRng) -> Self {
        let mut buffer = [0u8; 12];

        rng.fill_bytes(&mut buffer);

        Self {
            combined_random: buffer,
        }
    }
}

impl ConnectionId {
    /// Creates a new connection ID, based on random values from server and client, as well as
    /// node IDs.
    fn create(random_data: TlsRandomData, our_id: NodeId, their_id: NodeId) -> ConnectionId {
        // Hash the resulting random values.
        let mut id = hash::hash(random_data.combined_random).to_array();

        // We XOR in a hashes of server and client fingerprint, to ensure that in the case of an
        // accidental collision (e.g. when `server_random` and `client_random` turn out to be all
        // zeros), we still have a chance of producing a reasonable ID.
        if let Some(our_id_bytes) = our_id.hash_bytes() {
            utils::xor(&mut id, &our_id_bytes[0..Digest::LENGTH]);
        } else {
            error!(
                ?our_id,
                "small_network attempted to retrieve bytes of ID, but seems to be libp2p ID?"
            )
        }

        if let Some(their_id_bytes) = their_id.hash_bytes() {
            utils::xor(&mut id, &their_id_bytes[0..Digest::LENGTH]);
        } else {
            error!(
                ?their_id,
                "small_network attempted to retrieve bytes of ID, but seems to be libp2p ID?"
            )
        }

        ConnectionId(id)
    }

    /// Creates a new [`TraceID`] based on the message count.
    ///
    /// The `flag` should be created using the [`Role::in_flag`] or [`Role::out_flag`] method and
    /// must be created accordingly (`out_flag` when serializing, `in_flag` when deserializing).
    fn create_trace_id(&self, flag: u8, count: u64) -> TraceId {
        // Copy the basic network ID.
        let mut buffer = self.0;

        // Direction set on first byte.
        buffer[0] ^= flag;

        // XOR in message count.
        utils::xor(&mut buffer[4..12], &count.to_ne_bytes());

        // Hash again and truncate.
        let full_hash = hash::hash(&buffer);

        // Safe to expect here, as we assert earlier that `Digest` is at least 12 bytes.
        let truncated =
            TryFrom::try_from(&full_hash.to_array()[0..8]).expect("buffer size mismatch");

        TraceId(truncated)
    }

    #[inline]
    /// Returns a reference to the raw bytes of the connection ID.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Creates a new connection ID from an existing SSL connection.
    #[inline]
    pub(crate) fn from_connection(ssl: &SslRef, our_id: NodeId, their_id: NodeId) -> Self {
        Self::create(TlsRandomData::collect(ssl), our_id, their_id)
    }
}

/// Message sending direction.
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub(super) enum Role {
    /// Dialer, i.e. initiator of the connection.
    Dialer,
    /// Listener, acceptor of the connection.
    Listener,
}

impl Role {
    /// Returns a flag suitable for hashing incoming messages.
    #[inline]
    fn in_flag(self) -> u8 {
        !(self.out_flag())
    }

    /// Returns a flag suitable for hashing outgoing messages.
    #[inline]
    fn out_flag(self) -> u8 {
        // The magic flag uses 50% of the bits, to be XOR'd into the hash later.
        const MAGIC_FLAG: u8 = 0b10101010;

        match self {
            Role::Dialer => MAGIC_FLAG,
            Role::Listener => !MAGIC_FLAG,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::types::NodeId;

    use super::{ConnectionId, Role, TlsRandomData, TraceId};

    #[test]
    fn trace_id_has_16_character() {
        let data = [0, 1, 2, 3, 4, 5, 6, 7];

        let output = format!("{}", TraceId(data));

        assert_eq!(output.len(), 16);
    }

    #[test]
    fn can_create_deterministic_trace_id() {
        let mut rng = crate::new_rng();

        // Scenario: Nodes A and B are connecting to each other. Both connections are established.
        let node_a = NodeId::random(&mut rng);
        let node_b = NodeId::random(&mut rng);

        // We get two connections, with different Tls random data, but it will be the same on both
        // ends of the connection.
        let a_to_b_random = TlsRandomData::random(&mut rng);
        let a_to_b = ConnectionId::create(a_to_b_random, node_a, node_b);
        let a_to_b_alt = ConnectionId::create(a_to_b_random, node_b, node_a);

        // Ensure that either peer ends up with the same connection id.
        assert_eq!(a_to_b, a_to_b_alt);

        let b_to_a_random = TlsRandomData::random(&mut rng);
        let b_to_a = ConnectionId::create(b_to_a_random, node_b, node_a);
        let b_to_a_alt = ConnectionId::create(b_to_a_random, node_a, node_b);
        assert_eq!(b_to_a, b_to_a_alt);

        // The connection IDs must be distinct though.
        assert_ne!(a_to_b, b_to_a);

        // We are only looking at messages sent on the `a_to_b` connection, although from both ends.
        // In our example example, `node_a` is the dialing node, `node_b` the listener.

        // Trace ID on A, after sending to B.
        let msg_ab_0_on_a = a_to_b.create_trace_id(Role::Dialer.out_flag(), 0);

        // The same message on B.
        let msg_ab_0_on_b = a_to_b.create_trace_id(Role::Listener.in_flag(), 0);

        // These trace IDs must match.
        assert_eq!(msg_ab_0_on_a, msg_ab_0_on_b);

        // The second message must have a distinct trace ID.
        let msg_ab_1_on_a = a_to_b.create_trace_id(Role::Dialer.out_flag(), 1);
        let msg_ab_1_on_b = a_to_b.create_trace_id(Role::Listener.in_flag(), 1);
        assert_eq!(msg_ab_1_on_a, msg_ab_1_on_b);
        assert_ne!(msg_ab_0_on_a, msg_ab_1_on_a);

        // Sending a message on the **same connection** in a **different direction** also must yield
        // a different message id.
        let msg_ba_0_on_b = a_to_b.create_trace_id(Role::Listener.out_flag(), 0);
        let msg_ba_0_on_a = a_to_b.create_trace_id(Role::Dialer.in_flag(), 0);
        assert_eq!(msg_ba_0_on_b, msg_ba_0_on_a);
        assert_ne!(msg_ba_0_on_b, msg_ab_0_on_b);
    }
}
