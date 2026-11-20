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
    stream_types::{AdmissionWait, Event},
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
        let wait = self.wait;
        // A frame already buffered ends direct observation: the reader is
        // deliberately not polled again until that frame is dispatched, so
        // only the admission itself can settle this step.
        if self.inbound.pending.is_some() {
            return Ok(settle_admission(
                drive_pending_admission(state, wait, budget).await,
                Reader::Held,
            ));
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
            Watch::Admission(step) => settle_admission(step, Reader::Watched),
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

/// Whether the read half was still watched while the admission waited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reader {
    /// The admission raced the read half, so the next step polls it again.
    Watched,
    /// A buffered frame held the loop off the reader for this step.
    Held,
}

/// Translate a settled chunk admission into the loop's next action.
///
/// A chunk still queued when the wait expired does not end the step while the
/// reader was watched: the next step polls the reader again and resumes the
/// wait. With a frame already buffered the loop does not read, so the stall is
/// the only way back: the buffered frame cannot be dispatched until the chunk
/// settles, and the chunk cannot settle while the handler declines to read.
///
/// Not a `const fn`: `AdmissionStep` carries an owned reply, whose destructor
/// cannot be evaluated at compile time.
fn settle_admission<T>(settled: AdmissionStep, reader: Reader) -> AdmissionWait<T> {
    match (settled, reader) {
        (AdmissionStep::Handler(outcome), _) => AdmissionWait::Handler(outcome),
        (AdmissionStep::Continue, _) => AdmissionWait::Continue,
        (AdmissionStep::Stalled, Reader::Watched) => AdmissionWait::Continue,
        (AdmissionStep::Stalled, Reader::Held) => AdmissionWait::Stalled,
    }
}
