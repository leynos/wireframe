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

use std::{
    future::{self, Future},
    io,
};

use bytes::Bytes;
use futures::{Sink, Stream};
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        envelope::{Envelope, Packet},
        recovery_codec::DecodedItem,
    },
    frames::{InboundFrames, NextFrame},
    recovery::{RecoveryCoordinator, resolve_read_item},
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
    R: Stream<Item = io::Result<DecodedItem<F::Frame>>> + Unpin,
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
            let next = self.inbound.try_next(self.reader).await;
            return match next {
                Ok(NextFrame::TimedOut) => self.await_event(state, read_timeout_ms).await,
                Ok(next) => frame_event(next, self.inbound, &mut self.recovery, shutdown).await,
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
            recovery,
            ..
        } = self;
        let shutdown = self.shutdown;
        let push_lanes_open = actor.has_open_push_lanes();
        let read = read_event(inbound, reader, recovery, read_timeout_ms, shutdown);
        race_event(actor, state.streams.active.as_mut(), push_lanes_open, read).await
    }

    /// Read the next inbound frame and classify it for the loop.
    pub(super) async fn read_frame(&mut self, read_timeout_ms: u64) -> Event<F::Frame> {
        read_event(
            &mut *self.inbound,
            &mut *self.reader,
            &mut self.recovery,
            read_timeout_ms,
            self.shutdown,
        )
        .await
    }
}

/// Race queued pushes, the live handler, and the inbound read.
///
/// The handler branch is part of every race rather than a second select: with
/// no live exchange its future never resolves, and the select waits on the
/// other two exactly as it would with the branch omitted. Keeping one race
/// means the ordering that favours pushes cannot drift between the two cases.
async fn race_event<Fr, D>(
    actor: &mut ConnectionActor<Envelope, (), Fr>,
    active: Option<&mut ActiveStream>,
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
        outcome = poll_live(active) => Event::Handler(outcome),
        event = read => event,
    }
}

/// Poll the live handler, or wait forever when no body is live.
///
/// The never-ready future is what lets one race cover both cases: it occupies
/// the handler branch so the select never reports an exchange that is not
/// there.
async fn poll_live(active: Option<&mut ActiveStream>) -> Option<Bytes> {
    match active {
        Some(live) => live.poll_handler().await,
        None => future::pending().await,
    }
}

/// Wait for the reader, then classify what it produced.
async fn read_event<R, Fr>(
    inbound: &mut InboundFrames<DecodedItem<Fr>, Fr>,
    reader: &mut R,
    recovery: &mut RecoveryCoordinator<'_>,
    read_timeout_ms: u64,
    shutdown: &CancellationToken,
) -> Event<Fr>
where
    R: Stream<Item = io::Result<DecodedItem<Fr>>> + Unpin,
{
    let next = inbound.next_frame(reader, read_timeout_ms, shutdown).await;
    match next {
        Ok(next) => frame_event(next, inbound, recovery, shutdown).await,
        Err(error) => Event::Frame(Err(error)),
    }
}

/// Classify one read outcome for the loop.
///
/// A shutdown that arrives during the read closes it, so it is reported as a
/// stop rather than as a peer EOF: the peer may still be connected. A decoder
/// failure is resolved by [`resolve_read_item`]: a discarded or quarantined
/// failure resumes the read as an ordinary wake-up, and only a disconnect ends
/// the connection, with the decoder's own error.
async fn frame_event<T>(
    next: NextFrame<DecodedItem<T>>,
    inbound: &mut InboundFrames<DecodedItem<T>, T>,
    recovery: &mut RecoveryCoordinator<'_>,
    shutdown: &CancellationToken,
) -> Event<T> {
    match next {
        NextFrame::Ready(item) => {
            match resolve_read_item(item, inbound, recovery, shutdown).await {
                Ok(Some(frame)) => Event::Frame(Ok(Some(frame))),
                Ok(None) => Event::Tick,
                Err(error) => Event::Frame(Err(error)),
            }
        }
        NextFrame::Closed if shutdown.is_cancelled() => Event::Shutdown,
        NextFrame::Closed => Event::Frame(Ok(None)),
        NextFrame::TimedOut => Event::Tick,
    }
}
