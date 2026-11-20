//! Async polling utilities for the connection actor select loop.

use std::future::Future;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{ConnectionActor, output::ActiveOutput};
use crate::{push::FrameLike, response::FrameStream};

/// Borrowed event sources polled by the connection's select loop.
///
/// Splitting the borrow out of the select loop keeps that loop within the
/// project's cognitive-complexity limit and puts the actor's sources in one
/// place. At most one of `multi_packet` and `response` is `Some`, because
/// [`ActiveOutput`] keeps the output sources mutually exclusive.
pub(super) struct EventSources<'a, F, E, P> {
    /// Urgent push queue, polled ahead of the best-effort queue.
    pub(super) high: Option<&'a mut mpsc::Receiver<P>>,
    /// Best-effort push queue.
    pub(super) low: Option<&'a mut mpsc::Receiver<P>>,
    /// Active multi-packet channel, when that output source is active.
    pub(super) multi_packet: Option<&'a mut mpsc::Receiver<F>>,
    /// Active response stream, when that output source is active.
    pub(super) response: Option<&'a mut FrameStream<F, E>>,
    /// Connection shutdown signal.
    pub(super) shutdown: &'a CancellationToken,
    /// Peer half-close signal.
    pub(super) peer_closed: &'a CancellationToken,
}

impl<F, E, P> ConnectionActor<F, E, P> {
    /// Borrow the event sources polled by the next select loop iteration.
    pub(super) fn event_sources(&mut self) -> EventSources<'_, F, E, P> {
        let (multi_packet, response) = match &mut self.active_output {
            ActiveOutput::MultiPacket(ctx) => (ctx.channel_mut(), None),
            ActiveOutput::Response(stream) => (None, Some(stream)),
            ActiveOutput::None => (None, None),
        };
        EventSources {
            high: self.high_rx.as_mut(),
            low: self.low_rx.as_mut(),
            multi_packet,
            response,
            shutdown: &self.shutdown,
            peer_closed: &self.peer_closed,
        }
    }
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike,
{
    /// Await cancellation on the provided shutdown token.
    #[inline]
    pub(super) async fn wait_shutdown(token: &CancellationToken) { token.cancelled().await; }

    /// Await the peer closing its input half.
    #[inline]
    pub(super) async fn wait_peer_closed(token: &CancellationToken) { token.cancelled().await; }

    /// Poll `f` if `opt` is `Some`, returning `None` otherwise.
    pub(super) async fn poll_optional<'a, T, Fut, R>(
        opt: Option<&'a mut T>,
        f: impl FnOnce(&'a mut T) -> Fut + Send + 'a,
    ) -> Option<R>
    where
        T: Send + 'a,
        Fut: Future<Output = Option<R>> + Send + 'a,
    {
        if let Some(value) = opt {
            f(value).await
        } else {
            None
        }
    }

    /// Poll whichever receiver is provided, returning `None` when absent.
    ///
    /// Push queues and multi-packet channels reuse this helper so they share
    /// back-pressure with queued frames. `Q` is the channel item type: pushes
    /// and responses may differ when the server owns the connection.
    pub(super) async fn poll_queue<Q>(rx: Option<&mut mpsc::Receiver<Q>>) -> Option<Q>
    where
        Q: Send,
    {
        match rx {
            Some(receiver) => receiver.recv().await,
            None => None,
        }
    }
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike,
    E: std::fmt::Debug,
{
    /// Poll the streaming response.
    pub(super) async fn poll_response(
        resp: Option<&mut FrameStream<F, E>>,
    ) -> Option<Result<F, crate::response::WireframeError<E>>> {
        use futures::StreamExt;
        Self::poll_optional(resp, |s| s.next()).await
    }
}
