//! Packet abstraction and envelope types.
//!
//! This module defines the [`Packet`] trait along with [`Envelope`] and
//! [`PacketParts`] used to decompose and reassemble messages.

use crate::message::Message;

/// Envelope-like type used to wrap incoming and outgoing messages.
///
/// Custom envelope types must implement this trait so [`WireframeApp`] can
/// route messages and construct responses.
///
/// # Example
///
/// ```
/// use wireframe::{
///     app::{Packet, PacketParts},
///     message::Message,
/// };
///
/// #[derive(bincode::Decode, bincode::Encode)]
/// struct CustomEnvelope {
///     id: u32,
///     payload: Vec<u8>,
///     timestamp: u64,
/// }
///
/// impl Packet for CustomEnvelope {
///     fn id(&self) -> u32 { self.id }
///
///     fn correlation_id(&self) -> Option<u64> { None }
///
///     fn into_parts(self) -> PacketParts { PacketParts::new(self.id, None, self.payload) }
///
///     fn from_parts(parts: PacketParts) -> Self {
///         Self {
///             id: parts.id(),
///             payload: parts.payload(),
///             timestamp: 0,
///         }
///     }
/// }
/// ```
pub trait Packet: Message + Send + Sync + 'static {
    /// Return the message identifier used for routing.
    fn id(&self) -> u32;

    /// Return the correlation identifier tying this frame to a request.
    fn correlation_id(&self) -> Option<u64>;

    /// Consume the packet and return its identifier, correlation id and payload bytes.
    fn into_parts(self) -> PacketParts;

    /// Construct a new packet from raw parts.
    fn from_parts(parts: PacketParts) -> Self;
}

/// Component values extracted from or used to build a [`Packet`].
#[derive(Debug)]
pub struct PacketParts {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

/// Basic envelope type used by [`WireframeApp::handle_connection`].
///
/// Incoming frames are deserialized into an `Envelope` containing the
/// message identifier and raw payload bytes.
#[derive(bincode::Decode, bincode::Encode, Debug)]
pub struct Envelope {
    pub(crate) id: u32,
    pub(crate) correlation_id: Option<u64>,
    pub(crate) payload: Vec<u8>,
}

impl Envelope {
    /// Create a new [`Envelope`] with the provided identifiers and payload.
    #[must_use]
    pub fn new(id: u32, correlation_id: Option<u64>, payload: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            payload,
        }
    }
}

impl Packet for Envelope {
    #[inline]
    fn id(&self) -> u32 { self.id }

    #[inline]
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn into_parts(self) -> PacketParts { self.into() }

    fn from_parts(parts: PacketParts) -> Self { parts.into() }
}

impl PacketParts {
    /// Construct a new set of packet parts.
    #[must_use]
    pub fn new(id: u32, correlation_id: Option<u64>, payload: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            payload,
        }
    }

    #[must_use]
    pub const fn id(&self) -> u32 { self.id }

    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    #[must_use]
    pub fn payload(self) -> Vec<u8> { self.payload }

    /// Ensure a correlation identifier is present, inheriting from `source` if missing.
    ///
    /// # Examples
    /// ```
    /// use wireframe::app::PacketParts;
    /// // Inherit when missing
    /// let parts = PacketParts::new(1, None, vec![]).inherit_correlation(Some(42));
    /// assert_eq!(parts.correlation_id(), Some(42));
    ///
    /// // Overwrite mismatched value
    /// let parts = PacketParts::new(1, Some(7), vec![]).inherit_correlation(Some(8));
    /// assert_eq!(parts.correlation_id(), Some(8));
    /// ```
    #[must_use]
    pub fn inherit_correlation(mut self, source: Option<u64>) -> Self {
        match (self.correlation_id, source) {
            (None, cid) => self.correlation_id = cid,
            (Some(cid), Some(src)) if cid != src => {
                tracing::warn!(
                    id = self.id,
                    expected = src,
                    found = cid,
                    "mismatched correlation id in response",
                );
                // Overwrite with the source correlation ID to ensure downstream
                // consistency.
                self.correlation_id = Some(src);
            }
            _ => {}
        }
        self
    }
}

impl From<Envelope> for PacketParts {
    fn from(e: Envelope) -> Self { PacketParts::new(e.id, e.correlation_id, e.payload) }
}

impl From<PacketParts> for Envelope {
    fn from(p: PacketParts) -> Self {
        let id = p.id();
        let correlation_id = p.correlation_id();
        let payload = p.payload();
        Envelope::new(id, correlation_id, payload)
    }
}
