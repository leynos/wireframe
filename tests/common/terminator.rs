//! Test protocol that appends a terminator frame at end-of-stream.
//!
//! Used across integration and behavioural tests verifying the stream
//! termination mechanism.
use wireframe::hooks::{ConnectionContext, WireframeProtocol};

/// Protocol that produces `0` as an explicit end-of-stream marker.
pub struct Terminator;

impl WireframeProtocol for Terminator {
    type Frame = u8;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { Some(0) }
}
