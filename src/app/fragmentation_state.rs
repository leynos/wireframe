//! Connection-scoped helpers for outbound fragmentation and inbound reassembly.
//!
//! This module encapsulates the fragmentation state used by `ConnectionActor`
//! and `WireframeApp` to keep the main connection module concise.

use bincode::error::DecodeError;
use thiserror::Error;

use super::{Packet, PacketParts};
use crate::fragment::{
    Fragmentable,
    FragmentationError,
    Fragmenter,
    MessageId,
    Reassembler,
    ReassemblyError,
    decode_fragment_payload,
};

/// Bundles outbound fragmentation and inbound reassembly state for a connection.
pub(crate) struct FragmentationState {
    fragmenter: Fragmenter,
    reassembler: Reassembler,
}

/// Decode or reassembly failures encountered while handling fragments.
#[derive(Debug, Error)]
pub(crate) enum FragmentProcessError {
    #[error("decode error: {0}")]
    Decode(DecodeError),
    #[error("reassembly error: {0}")]
    Reassembly(ReassemblyError),
}

impl FragmentationState {
    pub(crate) fn new(config: crate::fragment::FragmentationConfig) -> Self {
        Self {
            fragmenter: Fragmenter::new(config.fragment_payload_cap),
            reassembler: Reassembler::new(config.max_message_size, config.reassembly_timeout),
        }
    }

    pub(crate) fn fragment<E: Fragmentable>(
        &self,
        packet: E,
    ) -> Result<Vec<E>, FragmentationError> {
        crate::fragment::fragment_packet(&self.fragmenter, packet)
    }

    pub(crate) fn reassemble<E: Packet>(
        &mut self,
        packet: E,
    ) -> Result<Option<E>, FragmentProcessError> {
        let parts = packet.into_parts();
        let id = parts.id();
        let correlation = parts.correlation_id();
        let payload = parts.payload();

        match decode_fragment_payload(&payload) {
            Ok(Some((header, fragment_payload))) => {
                match self.reassembler.push(header, fragment_payload) {
                    Ok(Some(message)) => {
                        let rebuilt = PacketParts::new(id, correlation, message.into_payload());
                        Ok(Some(E::from_parts(rebuilt)))
                    }
                    Ok(None) => Ok(None),
                    Err(err) => Err(FragmentProcessError::Reassembly(err)),
                }
            }
            Ok(None) => Ok(Some(E::from_parts(PacketParts::new(
                id,
                correlation,
                payload,
            )))),
            Err(err) => Err(FragmentProcessError::Decode(err)),
        }
    }

    pub(crate) fn purge_expired(&mut self) -> Vec<MessageId> { self.reassembler.purge_expired() }
}
