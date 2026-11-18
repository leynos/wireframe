//! Per-step maintenance actions for the streaming connection loop.
//!
//! [`super::stream_dispatch`] decides *when* each action runs; this module
//! holds the actions themselves. They are separate because each answers a
//! different question — has an assembly expired, is the connection over
//! budget, is a parked message now servable, how does the connection end — and
//! a loop that carried both the state machine and every action in one module
//! grew past the repository's module limit.
//!
//! Every action borrows the same [`DispatchHandles`] bundle, so a reply
//! produced by any of them travels the identical pipeline.

use std::{io, time::Duration};

use bytes::Bytes;
use futures::Sink;

use super::{
    super::{
        envelope::{Envelope, Packet},
        frame_handling::{self, Dispatch, MemoryPressureAction},
    },
    stream_dispatch::Step,
    stream_handles::{ChunkBudget, DispatchHandles, LoopState},
    stream_parked::{dispatch_parked, parked_bytes},
    stream_slot::PendingEvent,
};
use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::StreamingAssemblyId,
    serializer::Serializer,
};

/// Translate a response outcome into the loop's next decision.
pub(super) const fn finish_step(dispatch: Dispatch) -> Step {
    match dispatch {
        Dispatch::Continue => Step::Continue,
        Dispatch::Stop => Step::Closed,
    }
}

/// What one drive of the pending chunk admission decided.
pub(super) enum AdmissionStep {
    /// The admission settled, or stayed queued with the slot still live.
    Continue,
    /// The handler resolved first, so its reply is now owed.
    Handler(Option<Bytes>),
}

/// Advance the in-flight chunk admission, or report the handler if it resolves
/// first.
///
/// A handler that resolves here still owes a reply, and that reply may stream
/// for an unbounded time. The outcome is handed back rather than forwarded in
/// this scope because only the caller owns the reader, and is therefore the
/// only place that can forward the reply while still observing the peer.
pub(super) async fn drive_pending_admission(
    state: &mut LoopState,
    wait: Duration,
    budget: ChunkBudget,
) -> AdmissionStep {
    match state.streams.drive_pending(wait, budget).await {
        PendingEvent::Send(result) => state.streams.complete_send(result),
        PendingEvent::Handler(outcome) => return AdmissionStep::Handler(outcome),
        // The admission stays queued; the next step re-checks expiry and the
        // memory budget before resuming it.
        PendingEvent::Idle => {}
    }
    AdmissionStep::Continue
}

/// Purge expired assemblies and report the streaming bodies that must fail.
///
/// The reply an expired body produces is written by the caller through the
/// peer-observed path: it may stream for an unbounded time, so writing it here
/// would hide a peer that has already gone away.
pub(super) fn expire_finished_assemblies<S, E, K, F>(
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> Vec<StreamingAssemblyId>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
{
    let purged = frame_handling::purge_expired_assemblies(handles.assembly);
    handles.pipeline.purge_expired();
    purged.streaming().to_vec()
}

/// Abort the connection when retained bytes exceed the hard cap.
pub(super) async fn apply_memory_pressure<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
{
    let pressure = frame_handling::evaluate_memory_pressure_with_retained(
        handles.assembly.as_ref(),
        Some(handles.ctx.effective_budgets),
        handles.pipeline.fragmentation_buffered_bytes(),
        state
            .streams
            .retained_bytes()
            .saturating_add(parked_bytes(&state.parked)),
    );
    if matches!(pressure, MemoryPressureAction::Abort) {
        state.streams.abort();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "per-connection memory budget hard cap exceeded",
        ));
    }
    frame_handling::apply_memory_pressure(pressure, || {}).await
}

/// Dispatch every parked message while the slot is free.
pub(super) async fn drain_parked<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let mut dispatch = Dispatch::Continue;
    while let Some(message) = state.parked.pop_front() {
        // The messages still queued behind this one keep their bytes
        // reserved while it is answered.
        dispatch = dispatch_parked(message, &state.parked, handles.reborrow()).await?;
        if dispatch == Dispatch::Stop {
            break;
        }
    }
    Ok(dispatch)
}

/// Fail the live body and report whether the connection may still write.
///
/// Closing the body also forwards any reply the handler managed to produce, so
/// the result carries that delivery's outcome. A cancelled reply can leave a
/// partial frame on the wire, and [`Dispatch::Stop`] says no later write may
/// follow it; the caller must not serve anything else on that connection.
pub(super) async fn fail_body_ending_with<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
    reason: io::Error,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    state
        .streams
        .close_active(Some(reason), handles.reborrow())
        .await
}
