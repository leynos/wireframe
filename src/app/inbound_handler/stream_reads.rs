//! Waiting on the next unit of work for a streaming connection.
//!
//! The loop races three sources: queued pushes, the live handler resolving, and
//! the inbound read. Keeping that race here leaves
//! [`super::stream_dispatch`] as the decision of what a settled event means,
//! while this module owns how the wait is composed and how a read outcome is
//! classified.
//!
//! The reader, the sink, and the actor are borrowed separately, so the push
//! lanes and the read half are polled by the same select without borrowing the
//! whole connection. A frame buffered during dispatch is served first, and the
//! reader gets one non-blocking attempt after each push batch, so an inbound
//! frame waits behind at most one batch.

use std::{future::Future, io};

use futures::{Sink, Stream};
use tokio_util::sync::CancellationToken;

use super::{
    super::envelope::{Envelope, Packet},
    frames::{InboundFrames, NextFrame},
    stream_active::ActiveStream,
    stream_dispatch::Connection,
    stream_handles::LoopState,
    stream_pushes::push_event,
    stream_types::Event,
};
use crate::{
    codec::FrameCodec,
    connection::ConnectionActor,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

impl<S, E, R, K, F> Connection<'_, S, E, R, K, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Await the next unit of work for this connection.
    ///
    /// Three sources are raced in one biased select: queued pushes, the live
    /// handler resolving, and the inbound read. Pushes win ties, matching the
    /// connection actor's own priority ordering, so a queued push is not held
    /// behind a client that keeps its socket busy. Two rules bound that bias: a
    /// frame buffered during dispatch is served first, and after each push
    /// batch the reader gets one non-blocking attempt, so an inbound frame
    /// waits behind at most one batch.
    pub(super) async fn next_event(&mut self, state: &mut LoopState) -> Event<F::Frame> {
        let read_timeout_ms = self.ctx.read_timeout_ms;
        let shutdown = self.shutdown;
        // The probe is the reader's turn, so it is spent whether or not it
        // produces a frame.
        if std::mem::take(&mut state.serve_read_first) {
            return match self.inbound.try_next(self.reader).await {
                Ok(NextFrame::TimedOut) => self.await_event(state, read_timeout_ms).await,
                Ok(next) => frame_event(next, shutdown),
                Err(error) => Event::Frame(Err(error)),
            };
        }
        self.await_event(state, read_timeout_ms).await
    }

    /// Race the push lanes, the live handler, and the inbound read.
    ///
    /// The reader, the sink, and the actor are borrowed separately here, so a
    /// push lane and the read half are polled by the same select without
    /// borrowing the whole connection.
    async fn await_event(
        &mut self,
        state: &mut LoopState,
        read_timeout_ms: u64,
    ) -> Event<F::Frame> {
        let Connection {
            reader,
            inbound,
            actor,
            ..
        } = self;
        let shutdown = self.shutdown;
        let push_lanes_open = actor.has_open_push_lanes();
        let read = read_event(inbound, reader, read_timeout_ms, shutdown);
        match state.streams.active.as_mut() {
            Some(active) => race_live(actor, active, push_lanes_open, read).await,
            None => race_idle(actor, push_lanes_open, read).await,
        }
    }

    /// Read the next inbound frame and classify it for the loop.
    pub(super) async fn read_frame(&mut self, read_timeout_ms: u64) -> Event<F::Frame> {
        read_event(
            &mut *self.inbound,
            &mut *self.reader,
            read_timeout_ms,
            self.shutdown,
        )
        .await
    }
}

/// Race the live handler and the inbound read against queued pushes.
async fn race_live<Fr, D>(
    actor: &mut ConnectionActor<Envelope, (), Fr>,
    active: &mut ActiveStream,
    push_lanes_open: bool,
    read: D,
) -> Event<Fr>
where
    Fr: Send + 'static,
    D: Future<Output = Event<Fr>>,
{
    tokio::select! {
        biased;
        pushes = actor.next_pushes(), if push_lanes_open => push_event(pushes),
        outcome = active.poll_handler() => Event::Handler(outcome),
        event = read => event,
    }
}

/// Race the inbound read against queued pushes while no body is live.
async fn race_idle<Fr, D>(
    actor: &mut ConnectionActor<Envelope, (), Fr>,
    push_lanes_open: bool,
    read: D,
) -> Event<Fr>
where
    Fr: Send + 'static,
    D: Future<Output = Event<Fr>>,
{
    tokio::select! {
        biased;
        pushes = actor.next_pushes(), if push_lanes_open => push_event(pushes),
        event = read => event,
    }
}

/// Read one frame from the peer and classify its outcome for the loop.
async fn read_event<R, Fr>(
    inbound: &mut InboundFrames<Fr>,
    reader: &mut R,
    read_timeout_ms: u64,
    shutdown: &CancellationToken,
) -> Event<Fr>
where
    R: Stream<Item = io::Result<Fr>> + Unpin,
{
    match inbound.next_frame(reader, read_timeout_ms, shutdown).await {
        Ok(next) => frame_event(next, shutdown),
        Err(error) => Event::Frame(Err(error)),
    }
}

/// Classify one read outcome for the loop.
///
/// A shutdown that arrives during the read closes it, so it is reported as a
/// stop rather than as a peer EOF: the peer may still be connected.
fn frame_event<T>(next: NextFrame<T>, shutdown: &CancellationToken) -> Event<T> {
    match next {
        NextFrame::Ready(frame) => Event::Frame(Ok(Some(frame))),
        NextFrame::Closed if shutdown.is_cancelled() => Event::Shutdown,
        NextFrame::Closed => Event::Frame(Ok(None)),
        NextFrame::TimedOut => Event::Tick,
    }
}
