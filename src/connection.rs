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
    queues: PushQueues<F>,
    response: Option<FrameStream<F, E>>, // current streaming response
    shutdown: CancellationToken,
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
        // If cancellation has already been requested, exit immediately. Nothing
        // will be drained and any streaming response is abandoned. This mirrors
        // a hard shutdown and is required for the tests.
        if self.shutdown.is_cancelled() {
            return Ok(());
        }

        let mut state = ActorState::new(self.response.is_none());

        while !state.is_done() {
            self.poll_sources(&mut state, out).await?;
        }

        Ok(())
    }

    async fn poll_sources(
        &mut self,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        tokio::select! {
            biased;

            () = self.shutdown.cancelled(), if !state.shutting_down => {
                state.shutting_down = true;
                self.start_shutdown(&mut state.resp_closed);
            }

            res = self.queues.high_priority_rx.recv(), if !state.push.high => {
                Self::handle_push(res, &mut state.push.high, out);
            }

            res = self.queues.low_priority_rx.recv(), if !state.push.low => {
                Self::handle_push(res, &mut state.push.low, out);
            }

            res = async {
                if let Some(stream) = &mut self.response {
                    stream.next().await
                } else {
                    None
                }
            }, if !state.shutting_down && !state.resp_closed => {
                Self::handle_response(res, &mut state.resp_closed, out)?;
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

struct PushClosed {
    high: bool,
    low: bool,
}

struct ActorState {
    push: PushClosed,
    resp_closed: bool,
    shutting_down: bool,
}

impl ActorState {
    fn new(resp_closed: bool) -> Self {
        Self {
            push: PushClosed {
                high: false,
                low: false,
            },
            resp_closed,
            shutting_down: false,
        }
    }

    fn is_done(&self) -> bool {
        let push_drained = self.push.high && self.push.low;
        push_drained && (self.resp_closed || self.shutting_down)
    }
}
