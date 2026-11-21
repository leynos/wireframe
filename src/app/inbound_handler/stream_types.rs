//! Value types shared by the connection loop and its maintenance actions.
//!
//! The loop, the actions it schedules, and the handle bundle they build all
//! name the same three types: the immutable inputs a connection dispatches
//! with, the outcome of one step, and the event that step consumed. They live
//! here rather than beside the loop so neither module grows past the
//! repository's size limit, and so a reader of either one sees the shared
//! contract without a circular import.
//!
//! Only the types live here. The state machine that produces and consumes them
//! is in [`super::stream_dispatch`], and the actions it schedules are in
//! [`super::stream_actions`].

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use bytes::Bytes;

use super::super::{
    envelope::Packet,
    memory_budgets::MemoryBudgets,
    middleware_types::StreamingHandler,
};
use crate::{
    codec::FrameCodec,
    connection::PendingPushes,
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::Serializer,
};

/// Immutable connection inputs needed by the dispatch loop.
pub(super) struct DispatchContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch buffered envelopes.
    pub(super) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Streaming handlers keyed by message identifier.
    pub(super) streaming_routes: &'a HashMap<u32, StreamingHandler>,
    /// Serializer shared by all frames on the connection.
    pub(super) serializer: &'a S,
    /// Codec configuration shared by all frames on the connection.
    pub(super) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(super) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Protocol whose terminator and completion hooks follow a response.
    pub(super) protocol:
        Option<&'a Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Effective byte budgets for this connection.
    pub(super) effective_budgets: MemoryBudgets,
    /// Maximum interval to wait for the next inbound frame.
    pub(super) read_timeout_ms: u64,
}

/// What the loop should do after one step.
///
/// Shared with [`super::stream_actions`], whose actions can end the loop.
pub(super) enum Step {
    /// Keep reading frames.
    Continue,
    /// Stop dispatching: the peer closed, or a reply left its stream open.
    Closed,
    /// The connection must end with this error.
    Failed(io::Error),
}

/// How long a body chunk may wait for admission before its body is abandoned.
///
/// A chunk that cannot clear admission is retried on every step, so the loop
/// stays responsive, but nothing inside the connection ends the wait when the
/// handler never reads: the reader is held so the peer cannot push past the
/// byte budget, and no later frame can be dispatched until the chunk settles.
/// The bound is that exit. It matches the default message-assembly timeout, so
/// an abandoned body is bounded by the same interval as the partial messages
/// it arrived with, and it is long enough that a handler pausing for a slow
/// downstream call still keeps its upload.
pub(super) const STALLED_BODY_BOUND: Duration = Duration::from_secs(30);

/// What the loop learned while a chunk waited for admission.
///
/// The wait normally ends with either a settled admission or the handler
/// resolving. It also watches the read half: a chunk that cannot be admitted
/// keeps the loop off the reader, so nothing else would notice the peer
/// leaving while the budget stayed saturated.
pub(super) enum AdmissionWait<T> {
    /// Keep looping: the chunk settled, stayed queued, or a frame arrived.
    Continue,
    /// The handler resolved first, so its reply is owed.
    Handler(Option<Bytes>),
    /// The read half ended, failed, or the server stopped.
    Peer(Event<T>),
}

/// One step of the connection loop.
pub(super) enum Event<T> {
    /// An inbound frame, EOF, or decoder error.
    Frame(io::Result<Option<T>>),
    /// The server stopped the connection.
    Shutdown,
    /// The read timeout elapsed with no frame.
    Tick,
    /// The handler resolved while the loop was waiting.
    Handler(Option<Bytes>),
    /// Frames the connection actor took from the push queues.
    ///
    /// The batch is owned rather than borrowed so the wait can end before the
    /// sink is needed: a push write holds the sink and the reader together,
    /// which the wait itself cannot.
    Pushes(PendingPushes<T>),
    /// Every push lane closed, so only the inbound read remains.
    LanesIdle,
}
