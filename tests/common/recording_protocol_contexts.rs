//! Context-observation accessor for the recording protocol fixture.
//!
//! `RecordingProtocol` records the connection state every callback observed.
//! This accessor lives in its own module so a binary that only asserts dispatch
//! order does not compile a reader it never calls. Include this module after
//! `recording_protocol` with
//! `#[path = "common/recording_protocol_contexts.rs"] mod recording_protocol_contexts;`.

use std::sync::PoisonError;

use super::recording_protocol::{ObservedContext, RecordingProtocolFixture};

/// Snapshot the context observations recorded so far, in call order.
pub fn contexts<F>(fixture: &RecordingProtocolFixture<F>) -> Vec<ObservedContext> {
    fixture
        .contexts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
