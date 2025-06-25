//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
    push::{FrameLike, PushQueues},
    response::{FrameStream, WireframeError},
};

/// Actor driving outbound frame delivery for a connection.
pub struct ConnectionActor<F, E> {
    pub queues: PushQueues<F>,
    pub response: Option<FrameStream<F, E>>, // current streaming response
    pub shutdown: CancellationToken,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike,
{
    /// Create a new `ConnectionActor` from the provided components.
    #[must_use]
    pub fn new(
        queues: PushQueues<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            queues,
            response,
            shutdown,
        }
    }

    /// Drive the actor until all sources are exhausted or shutdown is triggered.
    ///
    /// Frames are appended to `out` in the order they are processed.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the response stream yields an error.
    pub async fn run(&mut self, out: &mut Vec<F>) -> Result<(), WireframeError<E>> {
        // If cancellation has already been requested, exit immediately.
        // Nothing will be drained, queued frames are lost and any streaming
        // response is abandoned. This mirrors a hard shutdown and is required
        // for the tests.
        if self.shutdown.is_cancelled() {
            return Ok(());
        }

        let mut high_closed = false;
        let mut low_closed = false;
        let mut resp_closed = self.response.is_none();
        let mut shutting_down = false;

        loop {
            tokio::select! {
                biased;

                () = self.shutdown.cancelled(), if !shutting_down => {
                    shutting_down = true;
                    self.start_shutdown(&mut resp_closed);
                }

                res = self.queues.high_priority_rx.recv(), if !high_closed => {
                    Self::handle_push(res, &mut high_closed, out);
                }

                res = self.queues.low_priority_rx.recv(), if !low_closed => {
                    Self::handle_push(res, &mut low_closed, out);
                }

                res = async {
                    if let Some(stream) = &mut self.response {
                        stream.next().await
                    } else {
                        None
                    }
                }, if !shutting_down && !resp_closed => {
                    Self::handle_response(res, &mut resp_closed, out)?;
                }
            }

            if Self::is_done(high_closed, low_closed, resp_closed, shutting_down) {
                break;
            }
        }

        Ok(())
    }

    fn start_shutdown(&mut self, resp_closed: &mut bool) {
        self.queues.high_priority_rx.close();
        self.queues.low_priority_rx.close();
        // Drop any streaming response so shutdown is prompt. Queued frames are
        // still drained, but streamed responses may be truncated.
        self.response = None;
        *resp_closed = true;
    }

    #[allow(clippy::fn_params_excessive_bools)]
    fn is_done(
        high_closed: bool,
        low_closed: bool,
        resp_closed: bool,
        shutting_down: bool,
    ) -> bool {
        let push_drained = high_closed && low_closed;
        push_drained && (resp_closed || shutting_down)
    }

    fn handle_push(res: Option<F>, closed: &mut bool, out: &mut Vec<F>) {
        match res {
            Some(frame) => out.push(frame),
            None => *closed = true,
        }
    }

    fn handle_response(
        res: Option<Result<F, WireframeError<E>>>,
        closed: &mut bool,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        match res {
            Some(Ok(frame)) => out.push(frame),
            Some(Err(e)) => return Err(e),
            None => *closed = true,
        }

        Ok(())
    }
}
