//! A protocol double that records every hook call, in order.
//!
//! Tests use this fixture to prove two different properties: that each
//! packet-oriented outbound path runs `before_send` exactly once per emitted
//! frame, and that a protocol-backed connection owns exactly one strong
//! reference to the installed protocol. This module provides the recorder and
//! its event log; the ownership probes live in
//! `recording_protocol_ownership` so a binary that only asserts dispatch order
//! does not compile probes it never uses. It is not part of any test target on
//! its own; include it with
//! `#[path = "common/recording_protocol.rs"] mod recording_protocol;`.

use std::{
    fmt::Debug,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use wireframe::{
    codec::EofError,
    hooks::{ConnectionContext, WireframeProtocol},
    push::PushHandle,
};

/// Byte appended by [`RecordingProtocol::before_send`] to mark a frame.
pub const MARKER: u8 = 0xb7;

/// One protocol callback observed by [`RecordingProtocol`], in call order.
///
/// `BeforeSend` carries the frame as the hook received it, before the marker
/// was appended, so assertions see exactly the frame the producer emitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEvent<F> {
    /// `on_connection_setup` ran.
    Setup,
    /// `before_send` observed `frame`.
    BeforeSend(F),
    /// `on_command_end` ran.
    CommandEnd,
    /// `handle_error` ran for a protocol error.
    ProtocolError,
    /// `stream_end_frame` ran.
    StreamEnd,
    /// `on_eof` ran with `error` and `partial`.
    Eof {
        /// The end-of-file condition the connection reported.
        error: EofError,
        /// Payload bytes received before the close.
        partial: Vec<u8>,
    },
}

/// Frame types the recorder can mark.
///
/// The marker makes hook mutation observable in the frames a peer decodes,
/// while the recorded event keeps the frame as it was before mutation.
pub trait MarkFrame: Clone + Debug + PartialEq + Eq + Send + Sync + 'static {
    /// Append [`MARKER`] to this frame.
    fn mark(&mut self, marker: u8);
}

impl MarkFrame for Vec<u8> {
    fn mark(&mut self, marker: u8) { self.push(marker); }
}

impl MarkFrame for Bytes {
    fn mark(&mut self, marker: u8) {
        let mut bytes = self.to_vec();
        bytes.push(marker);
        *self = Self::from(bytes);
    }
}

/// A [`WireframeProtocol`] that records hook calls and marks outbound frames.
pub struct RecordingProtocol<F> {
    /// Ordered hook events, shared with the fixture that created this value.
    events: Arc<Mutex<Vec<HookEvent<F>>>>,
    /// Counts drops of installed protocol instances.
    drops: Arc<AtomicUsize>,
    /// Frame returned by `stream_end_frame`, if any.
    terminator: Option<F>,
}

impl<F> Drop for RecordingProtocol<F> {
    fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); }
}

impl<F> RecordingProtocol<F>
where
    F: MarkFrame,
{
    /// Record one event.
    fn record(&self, event: HookEvent<F>) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

impl<F> WireframeProtocol for RecordingProtocol<F>
where
    F: MarkFrame,
{
    type Frame = F;
    type ProtocolError = ();

    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {
        self.record(HookEvent::Setup);
    }

    fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {
        self.record(HookEvent::BeforeSend(frame.clone()));
        frame.mark(MARKER);
    }

    fn on_command_end(&self, _ctx: &mut ConnectionContext) { self.record(HookEvent::CommandEnd); }

    fn handle_error(&self, (): Self::ProtocolError, _ctx: &mut ConnectionContext) {
        self.record(HookEvent::ProtocolError);
    }

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        self.record(HookEvent::StreamEnd);
        self.terminator.clone()
    }

    fn on_eof(&self, error: &EofError, partial_data: &[u8], _ctx: &mut ConnectionContext) {
        self.record(HookEvent::Eof {
            error: *error,
            partial: partial_data.to_vec(),
        });
    }
}

/// Owner of the shared observation handles for one recording protocol.
///
/// The fixture never holds a [`RecordingProtocol`] instance itself, so the
/// drop counter reports exactly the installed instances a test creates.
pub struct RecordingProtocolFixture<F> {
    /// Ordered hook events recorded by every instance built from this fixture.
    events: Arc<Mutex<Vec<HookEvent<F>>>>,
    /// Drops of installed instances built from this fixture.
    pub(crate) drops: Arc<AtomicUsize>,
    /// Frame returned by `stream_end_frame`, if any.
    terminator: Option<F>,
}

impl<F> RecordingProtocolFixture<F>
where
    F: MarkFrame,
{
    /// Create a fixture whose protocol returns `terminator` from
    /// `stream_end_frame`.
    pub fn new(terminator: Option<F>) -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            drops: Arc::new(AtomicUsize::new(0)),
            terminator,
        }
    }

    /// Build a fresh protocol instance sharing this fixture's observation
    /// handles.
    pub fn protocol(&self) -> RecordingProtocol<F> {
        RecordingProtocol {
            events: Arc::clone(&self.events),
            drops: Arc::clone(&self.drops),
            terminator: self.terminator.clone(),
        }
    }

    /// Snapshot the events recorded so far.
    pub fn events(&self) -> Vec<HookEvent<F>> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
