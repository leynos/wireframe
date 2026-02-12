//! Public adapter contract for transport-level fragmentation and reassembly.
//!
//! [`FragmentAdapter`] captures the minimal behaviour required to split outbound
//! packets into transport fragments, rebuild inbound fragments into full
//! packets, and purge stale partial assemblies.

use bincode::error::DecodeError;
use thiserror::Error;

use super::{
    Fragmentable,
    FragmentationConfig,
    FragmentationError,
    Fragmenter,
    MessageId,
    Reassembler,
    ReassemblyError,
    decode_fragment_payload,
    fragment_packet,
    packet::FragmentParts,
};

/// Error returned by [`FragmentAdapter::reassemble`].
#[derive(Debug, Error)]
pub enum FragmentAdapterError {
    /// Fragment payload marker/header decoding failed.
    #[error("decode error: {0}")]
    Decode(DecodeError),
    /// Reassembly ordering or size validation failed.
    #[error("reassembly error: {0}")]
    Reassembly(ReassemblyError),
}

/// Adapter contract for transport-level fragmentation and reassembly.
pub trait FragmentAdapter<E: Fragmentable>: Send + Sync {
    /// Attempt to fragment a packet for outbound transport.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentationError`] when payload chunking or header encoding
    /// fails.
    fn fragment(&self, packet: E) -> Result<Vec<E>, FragmentationError>;

    /// Attempt to reassemble an inbound packet.
    ///
    /// Returns `Ok(Some(packet))` when a complete packet is available,
    /// `Ok(None)` when more fragments are required, and an error when decoding
    /// or reassembly invariants fail.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentAdapterError`] when fragment decoding fails or when
    /// ordering and size guarantees are violated.
    fn reassemble(&mut self, packet: E) -> Result<Option<E>, FragmentAdapterError>;

    /// Purge stale partial reassembly state.
    ///
    /// Returns the identifiers that were evicted.
    fn purge_expired(&mut self) -> Vec<MessageId>;
}

/// Default adapter backed by [`Fragmenter`] and [`Reassembler`].
#[derive(Debug)]
pub struct DefaultFragmentAdapter {
    fragmenter: Fragmenter,
    reassembler: Reassembler,
}

impl DefaultFragmentAdapter {
    /// Create a default adapter from fragmentation configuration.
    #[must_use]
    pub fn new(config: FragmentationConfig) -> Self {
        Self {
            fragmenter: Fragmenter::new(config.fragment_payload_cap),
            reassembler: Reassembler::new(config.max_message_size, config.reassembly_timeout),
        }
    }

    fn fragment_inner<E: Fragmentable>(&self, packet: E) -> Result<Vec<E>, FragmentationError> {
        fragment_packet(&self.fragmenter, packet)
    }

    fn reassemble_inner<E: Fragmentable>(
        &mut self,
        packet: E,
    ) -> Result<Option<E>, FragmentAdapterError> {
        let parts = packet.into_fragment_parts();
        let id = parts.id();
        let correlation_id = parts.correlation_id();
        let payload = parts.into_payload();

        match decode_fragment_payload(&payload) {
            Ok(Some((header, fragment_payload))) => {
                match self.reassembler.push(header, fragment_payload) {
                    Ok(Some(message)) => {
                        let rebuilt =
                            FragmentParts::new(id, correlation_id, message.into_payload());
                        Ok(Some(E::from_fragment_parts(rebuilt)))
                    }
                    Ok(None) => Ok(None),
                    Err(err) => Err(FragmentAdapterError::Reassembly(err)),
                }
            }
            Ok(None) => {
                let passthrough = FragmentParts::new(id, correlation_id, payload);
                Ok(Some(E::from_fragment_parts(passthrough)))
            }
            Err(err) => Err(FragmentAdapterError::Decode(err)),
        }
    }

    fn purge_expired_inner(&mut self) -> Vec<MessageId> { self.reassembler.purge_expired() }

    /// Fragment outbound packet data.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentationError`] when fragment emission fails.
    pub fn fragment<E: Fragmentable>(&self, packet: E) -> Result<Vec<E>, FragmentationError> {
        self.fragment_inner(packet)
    }

    /// Reassemble inbound packet data.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentAdapterError`] when decoding or reassembly fails.
    pub fn reassemble<E: Fragmentable>(
        &mut self,
        packet: E,
    ) -> Result<Option<E>, FragmentAdapterError> {
        self.reassemble_inner(packet)
    }

    /// Purge stale reassembly entries and return evicted identifiers.
    pub fn purge_expired(&mut self) -> Vec<MessageId> { self.purge_expired_inner() }
}

impl<E: Fragmentable> FragmentAdapter<E> for DefaultFragmentAdapter {
    fn fragment(&self, packet: E) -> Result<Vec<E>, FragmentationError> {
        self.fragment_inner(packet)
    }

    fn reassemble(&mut self, packet: E) -> Result<Option<E>, FragmentAdapterError> {
        self.reassemble_inner(packet)
    }

    fn purge_expired(&mut self) -> Vec<MessageId> { self.purge_expired_inner() }
}
