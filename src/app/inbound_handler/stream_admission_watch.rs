//! Watching the read half while a body chunk waits for admission.
//!
//! Admission is decided against a byte budget that only the handler can free,
//! so a chunk can wait for as long as the handler declines to take it. Waiting
//! without reading would hide a peer that has already gone away, and the
//! connection would then outlive it with its handler, its queued bytes and its
//! server drain still held.
//!
//! The wait lives apart from [`super::stream_dispatch`] because the loop's
//! state machine and the actions it schedules together outgrew the
//! repository's module limit.

use std::io;

use futures::{Sink, Stream};

use super::{
    super::envelope::{Envelope, Packet},
    stream_actions::{AdmissionStep, drive_pending_admission},
    stream_dispatch::Connection,
    stream_handles::{ChunkBudget, LoopState},
    stream_types::{AdmissionWait, Event, STALLED_BODY_BOUND},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    request::QueuedBytes,
    serializer::Serializer,
};

/// Which half of a pending-admission wait settled first.
///
/// The two are kept apart because they mean different things: an admission is
/// a decision about the live body, while a read event ends or continues the
/// connection's frame exchange.
enum Watch<T> {
    /// The chunk admission reached a decision.
    Admission(AdmissionStep),
    /// The read half produced an event first.
    Read(Event<T>),
}

impl<S, E, R, K, F> Connection<'_, S, E, R, K, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Drive the slot's in-flight chunk admission while the read half stays
    /// observed.
    ///
    /// A chunk that cannot be admitted keeps the loop off the reader: the byte
    /// budget stays saturated for as long as the handler declines to take its
    /// chunk, so a peer that leaves meanwhile would go unnoticed and the
    /// connection would outlive it. The read half is therefore watched for the
    /// end of the exchange, and a settled event ends the step through the same
    /// path a frame read outside an admission takes.
    ///
    /// An arriving frame is kept for the next step rather than dispatched here.
    /// A continuation offered while its own chunk is still queued is refused as
    /// an abandoned body, so the frame must wait until the admission settles.
    ///
    /// The budget is measured on every drive, not when the chunk was queued:
    /// bytes retained elsewhere can only be released between drives, and the
    /// step that expires assemblies may just have released them.
    pub(super) async fn drive_admission(
        &mut self,
        state: &mut LoopState,
        queued: QueuedBytes,
    ) -> io::Result<AdmissionWait<F::Frame>> {
        // A pending admission means the slot is live, so the parked drain
        // cannot run in this step. Driving it first keeps the drain outside
        // this borrow scope, where the reader stays reachable.
        if !state.streams.has_pending_send() {
            return Ok(AdmissionWait::Continue);
        }
        let budget = {
            let handles = self.handles(queued);
            ChunkBudget::measure(&handles, &state.parked)
        };
        // A single drive is capped by what remains of the stall bound as well
        // as by the connection wait: the bound is checked between drives, so a
        // long read timeout would otherwise let one drive hide a body whose
        // handler never reads.
        let remaining = state.streams.stall_remaining(STALLED_BODY_BOUND);
        let wait = self.wait.min(remaining);
        // A frame already buffered ends direct observation: the reader is
        // deliberately not polled again until that frame is dispatched, so
        // only the admission or a server shutdown can settle this step. A
        // stall that lasts past the bound is reported by
        // `ConnectionStreams::stall_expired`, which the loop checks before it
        // drives the admission.
        if self.inbound.pending.is_some() {
            let shutdown = self.shutdown;
            let settled = tokio::select! {
                biased;
                settled = drive_pending_admission(state, wait, budget) => settled,
                // The read half cannot observe a shutdown while a frame is
                // buffered, so the token is raced here rather than waited out.
                () = shutdown.cancelled() => {
                    return Ok(AdmissionWait::Peer(Event::Shutdown));
                }
            };
            return Ok(settle_admission(settled));
        }
        let settled = {
            let read_timeout_ms = self.ctx.read_timeout_ms;
            tokio::select! {
                biased;
                settled = drive_pending_admission(state, wait, budget) => {
                    Watch::Admission(settled)
                }
                event = self.read_frame(read_timeout_ms) => Watch::Read(event),
            }
        };
        Ok(match settled {
            // The admission won the race, so no read was in flight and the
            // buffered frame cannot have changed.
            Watch::Admission(step) => settle_admission(step),
            Watch::Read(Event::Frame(Ok(Some(frame)))) => {
                self.inbound.pending = Some(frame);
                AdmissionWait::Continue
            }
            // A read timeout is the loop's ordinary wake-up, so it resumes the
            // admission rather than ending anything.
            Watch::Read(Event::Tick) => AdmissionWait::Continue,
            Watch::Read(event) => AdmissionWait::Peer(event),
        })
    }
}

/// Translate a settled chunk admission into the loop's next action.
///
/// A chunk still queued when the wait expired resumes the wait: the next step
/// drives the admission again, after re-checking shutdown, expiry and the
/// memory budget, and the stall it accumulates is what eventually ends a body
/// whose handler never reads.
///
/// Not a `const fn`: `AdmissionStep` carries an owned reply, whose destructor
/// cannot be evaluated at compile time.
fn settle_admission<T>(settled: AdmissionStep) -> AdmissionWait<T> {
    match settled {
        AdmissionStep::Handler(outcome) => AdmissionWait::Handler(outcome),
        // A settled admission and an expired wait both continue the loop.
        AdmissionStep::Continue => AdmissionWait::Continue,
    }
}
