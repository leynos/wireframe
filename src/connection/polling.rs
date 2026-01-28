//! Async polling utilities for the connection actor select loop.

use std::future::Future;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::ConnectionActor;
use crate::{push::FrameLike, response::FrameStream};

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike,
{
    /// Await cancellation on the provided shutdown token.
    #[inline]
    pub(super) async fn wait_shutdown(token: CancellationToken) { token.cancelled_owned().await; }

    /// Receive the next frame from a push queue.
    #[inline]
    pub(super) async fn recv_push(rx: &mut mpsc::Receiver<F>) -> Option<F> { rx.recv().await }

    /// Poll `f` if `opt` is `Some`, returning `None` otherwise.
    #[expect(
        clippy::manual_async_fn,
        reason = "Generic lifetime requires explicit async move"
    )]
    pub(super) fn poll_optional<'a, T, Fut, R>(
        opt: Option<&'a mut T>,
        f: impl FnOnce(&'a mut T) -> Fut + Send + 'a,
    ) -> impl Future<Output = Option<R>> + Send + 'a
    where
        T: Send + 'a,
        Fut: Future<Output = Option<R>> + Send + 'a,
    {
        async move {
            if let Some(value) = opt {
                f(value).await
            } else {
                None
            }
        }
    }

    /// Await shutdown cancellation on the provided token.
    pub(super) async fn await_shutdown(token: CancellationToken) {
        Self::wait_shutdown(token).await;
    }

    /// Poll whichever receiver is provided, returning `None` when absent.
    ///
    /// Multi-packet channels reuse this helper so they share back-pressure with queued frames.
    pub(super) async fn poll_queue(rx: Option<&mut mpsc::Receiver<F>>) -> Option<F> {
        Self::poll_optional(rx, Self::recv_push).await
    }
}

impl<F, E> ConnectionActor<F, E>
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
