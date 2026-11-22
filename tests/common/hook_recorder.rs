//! A recording protocol fixture for end-to-end hook dispatch tests.
//!
//! Unlike the actor-level recorder, this fixture captures the push handle the
//! server hands to `on_connection_setup`. A test can therefore prove that
//! server-initiated pushes are written, and that a handle retained past the
//! connection reports closure. It is not part of any test target on its own;
//! include it with `#[path = "common/hook_recorder.rs"] mod hook_recorder;`.

use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use wireframe::{
    codec::EofError,
    hooks::{ConnectionContext, WireframeProtocol},
    push::PushHandle,
};

/// Byte the protocol appends to every outbound frame.
///
/// The marker makes hook mutation visible in the bytes the peer receives while
/// the recorded event keeps the frame exactly as the producer emitted it.
pub const MARKER: u8 = 0xb7;

/// One observed protocol callback, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// `on_connection_setup` ran.
    Setup,
    /// `before_send` observed this frame before marking it.
    BeforeSend(Vec<u8>),
    /// `stream_end_frame` ran.
    StreamEnd,
    /// `on_command_end` ran.
    CommandEnd,
    /// `handle_error` ran.
    ProtocolError,
    /// `on_eof` ran with this condition and partial data.
    Eof {
        /// The end-of-file condition the connection reported.
        error: EofError,
        /// Bytes received for the frame the peer abandoned.
        partial: Vec<u8>,
    },
}

/// Shared observation handles for the recording protocol.
#[derive(Default)]
pub struct HookLog {
    /// Ordered hook events.
    events: Mutex<Vec<Event>>,
    /// Handle captured by the most recent connection setup.
    handle: Mutex<Option<PushHandle<Bytes>>>,
}

impl HookLog {
    /// Record one event.
    pub fn record(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    /// Snapshot the events recorded so far.
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Clone the push handle captured by connection setup.
    ///
    /// Returns `None` until a connection has run its setup hook.
    pub fn push_handle(&self) -> Option<PushHandle<Bytes>> {
        self.handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Store the handle a connection's setup hook received.
    fn store_handle(&self, handle: PushHandle<Bytes>) {
        *self.handle.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
    }
}

/// Protocol that records every hook, captures the push handle, and marks every
/// outbound frame.
pub struct RecordingProtocol {
    /// Shared observation handles.
    log: Arc<HookLog>,
    /// Frame returned by `stream_end_frame`, if any.
    terminator: Option<Bytes>,
}

impl RecordingProtocol {
    /// Build a protocol that records into `log` and terminates streams with
    /// `terminator`.
    pub const fn new(log: Arc<HookLog>, terminator: Option<Bytes>) -> Self {
        Self { log, terminator }
    }
}

impl WireframeProtocol for RecordingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {
        self.log.record(Event::Setup);
        self.log.store_handle(handle);
    }

    fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {
        self.log.record(Event::BeforeSend(frame.to_vec()));
        let mut marked = frame.to_vec();
        marked.push(MARKER);
        *frame = Bytes::from(marked);
    }

    fn on_command_end(&self, _ctx: &mut ConnectionContext) { self.log.record(Event::CommandEnd); }

    fn handle_error(&self, (): Self::ProtocolError, _ctx: &mut ConnectionContext) {
        self.log.record(Event::ProtocolError);
    }

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        self.log.record(Event::StreamEnd);
        self.terminator.clone()
    }

    fn on_eof(&self, error: &EofError, partial_data: &[u8], _ctx: &mut ConnectionContext) {
        self.log.record(Event::Eof {
            error: *error,
            partial: partial_data.to_vec(),
        });
    }
}
