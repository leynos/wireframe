//! Draining a connection's push queues while a handler runs.
//!
//! The connection loop is the only consumer of a connection's push queues, so a
//! handler that fills a lane while it runs could otherwise wait for itself.
//! These helpers poll the queues alongside the handler and write each frame
//! they take under the same shutdown and idle-grace bounds the actor applies to
//! its own output. A push write can block on a peer that stopped reading while
//! still connected, so the handler keeps being polled during that write rather
//! than waiting for the transport.

use std::{future::Future, io, pin::Pin, time::Duration};

use futures::Sink;
use log::warn;
use tokio_util::sync::CancellationToken;

use super::ResponseContext;
use crate::{
    app::codec_driver::ActorFrameSink,
    codec::FrameCodec,
    connection::PushSink,
    serializer::Serializer,
};

/// Run the handler while the connection's push queues keep being drained.
///
/// The connection loop is the only consumer of the connection's push queues, so
/// a handler that awaits a push into a full lane would otherwise wait for
/// itself: the loop cannot poll the lanes again until this call returns. Polling
/// them alongside the handler hands the lane capacity back, and each batch is
/// written before the next select, so a slow handler cannot accumulate queued
/// pushes in memory and no frame is held longer than the handler run.
///
/// Returns `None` when a server shutdown or an unobservable peer abandons the
/// handler while one of its push frames is blocked. The tokens are taken once
/// per response rather than per select pass, because the actor keeps the only
/// other copy; the grace deadline is rebuilt per frame so it measures idleness
/// rather than the time the whole batch has taken.
pub(super) async fn run_handler_with_pushes<Resp, S, K, F>(
    ctx: &mut ResponseContext<'_, S, K, F>,
    call: impl Future<Output = Resp>,
) -> io::Result<Option<Resp>>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    let shutdown = ctx.actor.shutdown_token();
    let unobserved = ctx.actor.unobserved_token();
    let bounds = PushWriteBounds {
        shutdown: &shutdown,
        unobserved: &unobserved,
        grace: ctx.actor.peer_closed_grace(),
    };
    let mut lanes_open = ctx.actor.has_open_push_lanes();
    let mut handler = HandlerRun {
        call: std::pin::pin!(call),
        finished: None,
    };
    loop {
        let frames = tokio::select! {
            biased;
            response = handler.poll_while_running() => return Ok(Some(response)),
            received = ctx.actor.next_pushes(), if lanes_open => {
                let Some(frames) = received else {
                    lanes_open = false;
                    continue;
                };
                frames
            },
        };
        for frame in frames {
            if write_push_watching(ctx, frame, &mut handler, &bounds).await? == PushWrite::Abandoned
            {
                return Ok(None);
            }
        }
        // A response produced during a write is returned only once the frames
        // already taken from the lane are written, so no queued push is lost.
        if let Some(response) = handler.finished.take() {
            return Ok(Some(response));
        }
    }
}

/// The handler future plus a response it produced during a push write.
///
/// A push write can block on a peer that stopped reading while the peer is
/// still observed, so the handler keeps being polled during the write. Once it
/// completes, its response is stored here and the future is never polled
/// again, because polling a completed future is not allowed.
struct HandlerRun<'a, Fut: Future> {
    /// Handler future, polled until it completes.
    call: Pin<&'a mut Fut>,
    /// Response captured when the handler finished during a push write.
    finished: Option<Fut::Output>,
}

impl<Fut: Future> HandlerRun<'_, Fut> {
    /// Resolve with the handler's response unless it already finished.
    ///
    /// The future stays pending once the response is stored, so this can stay
    /// one arm of a `select!` without polling the completed handler again.
    async fn poll_while_running(&mut self) -> Fut::Output {
        if self.finished.is_some() {
            return std::future::pending().await;
        }
        self.call.as_mut().await
    }
}

/// Bounds applied to a push write made while a handler runs.
struct PushWriteBounds<'a> {
    /// Token cancelled when the server must stop.
    shutdown: &'a CancellationToken,
    /// Token cancelled once the peer can no longer be observed.
    unobserved: &'a CancellationToken,
    /// How long an unobservable peer may take to accept one frame.
    grace: Duration,
}

/// Whether a push write let the connection keep serving its peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PushWrite {
    /// The frame reached the transport.
    Written,
    /// The push was abandoned; the connection must stop.
    Abandoned,
}

/// Write one drained push frame while continuing to poll the handler.
///
/// A peer that is still observed can stop reading without closing, and the
/// transport write then waits for its receive window. Awaiting that write alone
/// would stop polling the handler until the server stops, so a handler timeout
/// or recovery path could not run. The write is therefore raced against the
/// handler: a response it produces first is stored, and the write still runs to
/// completion or to one of its bounds, because dropping an unfinished frame
/// write and reusing the transport would truncate the frame stream.
async fn write_push_watching<S, K, F, Fut>(
    ctx: &mut ResponseContext<'_, S, K, F>,
    frame: F::Frame,
    handler: &mut HandlerRun<'_, Fut>,
    bounds: &PushWriteBounds<'_>,
) -> io::Result<PushWrite>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Fut: Future,
{
    let writing = write_push(ctx, frame);
    tokio::pin!(writing);
    let deadline = grace_after_observation(bounds.unobserved, bounds.grace);
    tokio::pin!(deadline);
    loop {
        let response = tokio::select! {
            biased;
            // A ready write is preferred, matching the actor's own rule: output
            // the transport would take must not be discarded for a bound that
            // has already fired.
            result = &mut writing => {
                result?;
                return Ok(PushWrite::Written);
            }
            response = handler.poll_while_running() => Some(response),
            () = bounds.shutdown.cancelled() => return Ok(PushWrite::Abandoned),
            () = &mut deadline => {
                warn!("abandoning a push write: the peer stopped reading");
                return Ok(PushWrite::Abandoned);
            }
        };
        if let Some(pending) = response {
            handler.finished = Some(pending);
        }
    }
}

/// Deadline that starts only once the peer can no longer be observed.
///
/// Graceful drain lets a response finish after the peer half-closes, but a peer
/// that also stopped reading can hold a write open forever. The grace starts
/// when observation ends rather than when the write starts, because a peer
/// still watching deserves the time it takes to accept the frame. The timer is
/// therefore built only after the token is cancelled: a deadline created
/// earlier would already be part-way through its interval, or past it.
pub(super) async fn grace_after_observation(unobserved: &CancellationToken, grace: Duration) {
    unobserved.cancelled().await;
    tokio::time::sleep(grace).await;
}

/// Write one drained push frame through the connection's transport sink.
///
/// A push is already encoded for the wire, so the sink applies only the
/// `before_send` hook and the transport write.
async fn write_push<S, K, F>(
    ctx: &mut ResponseContext<'_, S, K, F>,
    frame: F::Frame,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    let mut sink = ActorFrameSink::new(
        ctx.serializer,
        ctx.codec,
        &mut *ctx.sink,
        &mut *ctx.pipeline,
    );
    sink.send_push(frame).await
}
