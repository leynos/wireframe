//! Packet abstraction and envelope types.
//!
//! These types decouple serialisation from routing by wrapping raw payloads in
//! identifiers understood by [`crate::app::builder::WireframeApp`]. This
//! allows the builder to route frames before full deserialisation. See
//! [`crate::app::builder::WireframeApp`] for how envelopes are used when
//! registering routes.

use crate::{
    correlation::CorrelatableFrame,
    fragment::{FragmentParts, Fragmentable},
    message::Message,
};

/// Envelope-like type used to wrap incoming and outgoing messages.
///
/// Custom envelope types must implement this trait so
/// [`crate::app::builder::WireframeApp`] can
/// route messages and construct responses.
///
/// # Example
///
/// ```
/// use wireframe::{
///     app::{Packet, PacketParts},
///     correlation::CorrelatableFrame,
///     message::Message,
/// };
///
/// #[derive(bincode::Decode, bincode::Encode)]
/// struct CustomEnvelope {
///     id: u32,
///     correlation_id: Option<u64>,
///     payload: Vec<u8>,
///     timestamp: u64,
/// }
///
/// // CorrelatableFrame is required by Packet.
/// impl CorrelatableFrame for CustomEnvelope {
///     fn correlation_id(&self) -> Option<u64> { self.correlation_id }
///
///     fn set_correlation_id(&mut self, correlation_id: Option<u64>) {
///         self.correlation_id = correlation_id;
///     }
/// }
///
/// impl Packet for CustomEnvelope {
///     fn id(&self) -> u32 { self.id }
///
///     fn into_parts(self) -> PacketParts {
///         PacketParts::new(self.id, self.correlation_id, self.payload)
///     }
///
///     fn from_parts(parts: PacketParts) -> Self {
///         Self {
///             id: parts.id(),
///             correlation_id: parts.correlation_id(),
///             payload: parts.into_payload(),
///             timestamp: 0,
///         }
///     }
/// }
/// ```
pub trait Packet: CorrelatableFrame + Message + Send + Sync + 'static {
    /// Return the message identifier used for routing.
    fn id(&self) -> u32;

    /// Consume the packet and return its identifier, correlation id and payload bytes.
    fn into_parts(self) -> PacketParts;

    /// Construct a new packet from raw parts.
    fn from_parts(parts: PacketParts) -> Self;
}

/// Component values extracted from or used to build a [`Packet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketParts {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

/// Basic envelope type used by
/// [`crate::app::builder::WireframeApp::handle_connection`].
///
/// Incoming frames are deserialised into an `Envelope` containing the
/// message identifier and raw payload bytes.
#[derive(bincode::Decode, bincode::Encode, Debug, Clone, PartialEq, Eq)]
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

    /// Borrow the raw payload bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::Envelope;
    ///
    /// let envelope = Envelope::new(1, None, vec![7, 8]);
    /// assert_eq!(envelope.payload_bytes(), &[7, 8]);
    /// ```
    #[must_use]
    pub fn payload_bytes(&self) -> &[u8] { &self.payload }
}

impl Packet for Envelope {
    #[inline]
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> PacketParts { self.into() }

    fn from_parts(parts: PacketParts) -> Self { parts.into() }
}

impl CorrelatableFrame for Envelope {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, correlation_id: Option<u64>) {
        self.correlation_id = correlation_id;
    }
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

    /// Return the message identifier used to route this frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::PacketParts;
    ///
    /// let parts = PacketParts::new(9, None, vec![1, 2, 3]);
    /// assert_eq!(parts.id(), 9);
    /// ```
    #[must_use]
    pub const fn id(&self) -> u32 { self.id }

    /// Retrieve the correlation identifier, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::PacketParts;
    ///
    /// let parts = PacketParts::new(1, Some(42), vec![]);
    /// assert_eq!(parts.correlation_id(), Some(42));
    /// ```
    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    /// Borrow the raw payload bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::PacketParts;
    ///
    /// let parts = PacketParts::new(1, None, vec![7, 8]);
    /// assert_eq!(parts.payload_bytes(), &[7, 8]);
    /// ```
    #[must_use]
    pub fn payload_bytes(&self) -> &[u8] { &self.payload }

    /// Consume the parts and return the raw payload bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::PacketParts;
    ///
    /// let parts = PacketParts::new(1, None, vec![7, 8]);
    /// assert_eq!(parts.into_payload(), vec![7, 8]);
    /// ```
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> { self.payload }

    /// Deprecated: use [`PacketParts::into_payload`] instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::PacketParts;
    ///
    /// let parts = PacketParts::new(1, None, vec![7, 8]);
    /// assert_eq!(parts.into_payload(), vec![7, 8]);
    /// ```
    #[deprecated(since = "0.2.0", note = "Use `PacketParts::into_payload` instead.")]
    #[must_use]
    pub fn payload(self) -> Vec<u8> { self.into_payload() }

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
        let (next, mismatched) = Self::select_correlation(self.correlation_id, source);
        if mismatched && let (Some(found), Some(expected)) = (self.correlation_id, next) {
            log::warn!(
                "mismatched correlation id in response: id={}, expected={}, found={}",
                self.id,
                expected,
                found
            );
        }
        self.correlation_id = next;
        self
    }

    #[inline]
    fn select_correlation(current: Option<u64>, source: Option<u64>) -> (Option<u64>, bool) {
        match (current, source) {
            (None, cid) => (cid, false),
            (Some(cid), Some(src)) if cid != src => (Some(src), true),
            (curr, _) => (curr, false),
        }
    }
}

impl From<Envelope> for PacketParts {
    fn from(e: Envelope) -> Self { PacketParts::new(e.id, e.correlation_id, e.payload) }
}

impl From<PacketParts> for Envelope {
    fn from(p: PacketParts) -> Self {
        let id = p.id();
        let correlation_id = p.correlation_id();
        let payload = p.into_payload();
        Envelope::new(id, correlation_id, payload)
    }
}

// Blanket implementation: any Packet is automatically Fragmentable.
impl<T: Packet> Fragmentable for T {
    fn into_fragment_parts(self) -> FragmentParts {
        let parts = self.into_parts();
        FragmentParts::new(parts.id(), parts.correlation_id(), parts.into_payload())
    }

    fn from_fragment_parts(parts: FragmentParts) -> Self {
        T::from_parts(PacketParts::new(
            parts.id(),
            parts.correlation_id(),
            parts.into_payload(),
        ))
    }
}
