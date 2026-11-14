//! Draining a connection's push queues while a handler runs.
//!
//! The connection loop is the only consumer of a connection's push queues, so a
//! handler that fills a lane while it runs could otherwise wait for itself.
//! These helpers poll the queues alongside the handler and write each batch
//! under the same shutdown and idle-grace bounds the actor applies to its own
//! output.

use std::{future::Future, io};

use futures::Sink;
use log::warn;
use tokio_util::sync::CancellationToken;

use super::ResponseContext;
use crate::{
    app::codec_driver::ActorFrameSink,
    codec::FrameCodec,
    connection::{PushSink, write_before_bound},
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
/// handler while one of its push batches is blocked. The tokens and the grace
/// deadline are taken once per response rather than per select pass, because
/// the actor keeps the only other copy.
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
    let mut lanes_open = ctx.actor.has_open_push_lanes();
    tokio::pin!(call);
    loop {
        tokio::select! {
            biased;
            response = &mut call => return Ok(Some(response)),
            received = ctx.actor.next_pushes(), if lanes_open => match received {
                Some(frames) => {
                    // This write is outside the actor's own bounds, so it is
                    // bounded the same way: a ready write wins, a blocked one
                    // is cut short by the stop token, and the idle grace
                    // bounds it once the peer can no longer be observed. A
                    // peer that half-closed and stopped reading would
                    // otherwise pin the connection and its server drain.
                    let grace = ctx.actor.peer_closed_grace();
                    let deadline = grace_after_observation(&unobserved, grace);
                    let bound =
                        write_before_bound(write_push_batch(ctx, frames), &shutdown, deadline)
                            .await?;
                    if bound.is_finished() {
                        continue;
                    }
                    if bound.is_grace_expired() {
                        warn!("abandoning a push write: the peer stopped reading");
                    }
                    return Ok(None);
                }
                None => lanes_open = false,
            },
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
pub(super) async fn grace_after_observation(
    unobserved: &CancellationToken,
    grace: std::time::Duration,
) {
    unobserved.cancelled().await;
    tokio::time::sleep(grace).await;
}

/// Write one drained push batch through the connection's transport sink.
///
/// A push is already encoded for the wire, so the sink applies only the
/// `before_send` hook and the transport write.
async fn write_push_batch<S, K, F>(
    ctx: &mut ResponseContext<'_, S, K, F>,
    frames: Vec<F::Frame>,
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
    for frame in frames {
        sink.send_push(frame).await?;
    }
    Ok(())
}
