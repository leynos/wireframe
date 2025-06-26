//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    hooks::ProtocolHooks,
    push::{FrameLike, PushQueues},
    response::{FrameStream, WireframeError},
};

/// Actor driving outbound frame delivery for a connection.
pub struct ConnectionActor<F, E> {
    queues: PushQueues<F>,
    response: Option<FrameStream<F, E>>, // current streaming response
    shutdown: CancellationToken,
    hooks: ProtocolHooks<F>,
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
        Self::with_hooks(queues, response, shutdown, ProtocolHooks::default())
    }

    /// Create a new `ConnectionActor` with custom protocol hooks.
    #[must_use]
    pub fn with_hooks(
        queues: PushQueues<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        hooks: ProtocolHooks<F>,
    ) -> Self {
        Self {
            queues,
            response,
            shutdown,
            hooks,
        }
    }

    /// Access the underlying push queues.
    ///
    /// This is mainly used in tests to close the queues when no actor is
    /// draining them.
    #[must_use]
    pub fn queues_mut(&mut self) -> &mut PushQueues<F> { &mut self.queues }

    /// Set or replace the current streaming response.
    pub fn set_response(&mut self, stream: Option<FrameStream<F, E>>) { self.response = stream; }

    /// Get a clone of the shutdown token used by the actor.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken { self.shutdown.clone() }

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

            () = Self::wait_shutdown(self.shutdown.clone()), if !state.shutting_down => {
                state.shutting_down = true;
                self.start_shutdown(&mut state.resp_closed);
            }

            res = Self::recv_push(&mut self.queues.high_priority_rx), if !state.push.high => {
                Self::handle_push(res, &mut state.push.high, out);
            }

            res = Self::recv_push(&mut self.queues.low_priority_rx), if !state.push.low => {
                Self::handle_push(res, &mut state.push.low, out);
            }

            res = Self::next_response(&mut self.response), if !state.shutting_down && !state.resp_closed => {
                Self::handle_response(res, &mut state.resp_closed, out)?;
                if state.resp_closed {
                    self.response = None;
                }
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

    fn handle_push(&mut self, res: Option<F>, closed: &mut bool, out: &mut Vec<F>) {
        match res {
            Some(mut frame) => {
                self.hooks.before_send(&mut frame);
                out.push(frame);
            }
            None => *closed = true,
        }
    }

    fn handle_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        closed: &mut bool,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        match res {
            Some(Ok(mut frame)) => {
                self.hooks.before_send(&mut frame);
                out.push(frame);
            }
            Some(Err(e)) => return Err(e),
            None => {
                *closed = true;
                self.hooks.on_command_end();
            }
        }

        Ok(())
    }

    #[inline]
    async fn wait_shutdown(token: CancellationToken) { token.cancelled_owned().await; }

    #[inline]
    async fn recv_push(rx: &mut mpsc::Receiver<F>) -> Option<F> { rx.recv().await }

    #[inline]
    async fn next_response(
        stream: &mut Option<FrameStream<F, E>>,
    ) -> Option<Result<F, WireframeError<E>>> {
        if let Some(s) = stream {
            s.next().await
        } else {
            None
        }
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
