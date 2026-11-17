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

use std::io;

use futures::Sink;

use super::{
    super::{
        envelope::{Envelope, Packet},
        frame_handling::{self, Dispatch, MemoryPressureAction},
    },
    stream_dispatch::Step,
    stream_handles::{DispatchHandles, LoopState},
    stream_parked::{dispatch_parked, parked_bytes},
    stream_slot::PendingEvent,
};
use crate::{codec::FrameCodec, message::EncodeWith, serializer::Serializer};

/// Translate a response outcome into the loop's next decision.
pub(super) const fn finish_step(dispatch: Dispatch) -> Step {
    match dispatch {
        Dispatch::Continue => Step::Continue,
        Dispatch::Stop => Step::Closed,
    }
}

/// Advance the in-flight chunk admission, or the handler if it resolves first.
pub(super) async fn drive_pending_admission<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    match state.streams.drive_pending(handles.wait).await {
        PendingEvent::Send(result) => state.streams.complete_send(result),
        PendingEvent::Handler(outcome) => {
            let dispatch = state
                .streams
                .finish_active(outcome, handles.reborrow())
                .await?;
            return Ok(finish_step(dispatch));
        }
        // The admission stays queued; the next step re-checks expiry and the
        // memory budget before resuming it.
        PendingEvent::Idle => {}
    }
    Ok(Step::Continue)
}

/// Fail bodies whose assemblies expired, then release the expired entries.
pub(super) async fn expire_finished_assemblies<S, E, K, F>(
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
    let purged = frame_handling::purge_expired_assemblies(handles.assembly);
    handles.pipeline.purge_expired();
    if purged.streaming().is_empty() {
        return Ok(Dispatch::Continue);
    }
    state
        .streams
        .expire(purged.streaming(), handles.reborrow())
        .await
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

/// Fail the live body, then end the loop with `step`.
async fn fail_body_ending_with<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
    reason: io::Error,
    step: Step,
) -> io::Result<Step>
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
        .await?;
    Ok(step)
}

/// Fail the live body, then end the connection with the decoder error.
pub(super) async fn fail_after_decode_error<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
    error: io::Error,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let reason = io::Error::new(
        error.kind(),
        "inbound frame decoding failed while a request body was open",
    );
    fail_body_ending_with(state, handles, reason, Step::Failed(error)).await
}

/// Fail the live body, answer parked messages, and end the connection cleanly.
pub(super) async fn close_after_eof<S, E, K, F>(
    state: &mut LoopState,
    handles: &mut DispatchHandles<'_, S, E, K, F>,
) -> io::Result<Step>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    Envelope: EncodeWith<S>,
{
    let reason = io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "connection closed before the request body completed",
    );
    fail_body_ending_with(state, handles, reason, Step::Closed).await?;
    drain_parked(state, handles).await?;
    Ok(Step::Closed)
}
