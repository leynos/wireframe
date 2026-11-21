//! Ownership probes for the recording protocol fixture.
//!
//! The recorder in `recording_protocol` observes hook dispatch. These helpers
//! observe lifetime: how many strong references an owner holds, whether a
//! connection released its reference, and how many installed instances were
//! dropped. Keeping them in their own module means a binary that only checks
//! dispatch order does not compile probes it never calls. Include this module
//! after `recording_protocol` with
//! `#[path = "common/recording_protocol_ownership.rs"] mod recording_protocol_ownership;`.

use std::sync::{Arc, Weak, atomic::Ordering};

use super::recording_protocol::{MarkFrame, RecordingProtocol, RecordingProtocolFixture};

/// Build a fresh protocol instance behind the single `Arc` a caller owns.
pub fn shared<F>(fixture: &RecordingProtocolFixture<F>) -> Arc<RecordingProtocol<F>>
where
    F: MarkFrame,
{
    Arc::new(fixture.protocol())
}

/// Number of installed protocol instances dropped so far.
pub fn drop_count<F>(fixture: &RecordingProtocolFixture<F>) -> usize {
    fixture.drops.load(Ordering::SeqCst)
}

/// Number of strong references currently keeping `protocol` alive.
pub fn strong_count<F>(protocol: &Arc<RecordingProtocol<F>>) -> usize {
    Arc::strong_count(protocol)
}

/// Capture a weak handle used to prove a connection released its reference.
pub fn weak_handle<F>(protocol: &Arc<RecordingProtocol<F>>) -> Weak<RecordingProtocol<F>> {
    Arc::downgrade(protocol)
}

/// `true` while some strong reference still keeps the pointee alive.
pub fn is_alive<T: ?Sized>(weak: &Weak<T>) -> bool { weak.strong_count() > 0 }
