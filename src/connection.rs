//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

use futures::StreamExt;
use tokio::{
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    hooks::ProtocolHooks,
    push::{FrameLike, PushQueues},
    response::{FrameStream, WireframeError},
};

/// Configuration controlling fairness when draining push queues.
#[derive(Clone, Copy)]
pub struct FairnessConfig {
    /// Number of consecutive high-priority frames to process before
    /// checking the low-priority queue.
    pub max_high_before_low: usize,
    /// A zero value disables the counter and relies solely on `time_slice` for
    /// fairness, preserving strict high-priority ordering otherwise.
    /// Optional time slice after which the low-priority queue is checked
    /// if high-priority traffic has been continuous.
    pub time_slice: Option<Duration>,
}

impl Default for FairnessConfig {
    fn default() -> Self {
        Self {
            max_high_before_low: 8,
            time_slice: None,
        }
    }
}

/// Actor driving outbound frame delivery for a connection.
pub struct ConnectionActor<F, E> {
    queues: PushQueues<F>,
    response: Option<FrameStream<F, E>>, // current streaming response
    shutdown: CancellationToken,
    hooks: ProtocolHooks<F>,
    fairness: FairnessConfig,
    high_counter: usize,
    high_start: Option<Instant>,
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
            fairness: FairnessConfig::default(),
            high_counter: 0,
            high_start: None,
        }
    }

    /// Access the underlying push queues.
    ///
    /// This is mainly used in tests to close the queues when no actor is
    /// draining them.
    #[must_use]
    pub fn queues_mut(&mut self) -> &mut PushQueues<F> { &mut self.queues }

    /// Replace the fairness configuration.
    pub fn set_fairness(&mut self, fairness: FairnessConfig) { self.fairness = fairness; }

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
                self.process_shutdown(state);
            }

            res = Self::recv_push(&mut self.queues.high_priority_rx), if !state.push.high => {
                self.process_high(res, state, out);
            }

            res = Self::recv_push(&mut self.queues.low_priority_rx), if !state.push.low => {
                self.process_low(res, state, out);
            }

            res = Self::next_response(&mut self.response), if !state.shutting_down && !state.resp_closed => {
                self.process_response(res, state, out)?;
            }
        }

        Ok(())
    }

    fn process_shutdown(&mut self, state: &mut ActorState) {
        state.shutting_down = true;
        self.start_shutdown(&mut state.resp_closed);
    }

    fn process_high(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        let processed = res.is_some();
        self.handle_push(res, &mut state.push.high, out);
        if processed {
            self.after_high(out, &mut state.push.low);
        } else {
            self.reset_high_counter();
        }
    }

    fn process_low(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        let processed = res.is_some();
        self.handle_push(res, &mut state.push.low, out);
        if processed {
            self.after_low();
        }
    }

    fn process_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        let processed = matches!(res, Some(Ok(_)));
        self.handle_response(res, &mut state.resp_closed, out)?;
        if processed {
            self.after_low();
        }
        if state.resp_closed {
            self.response = None;
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

    fn after_high(&mut self, out: &mut Vec<F>, low_closed: &mut bool) {
        self.high_counter += 1;
        if self.high_counter == 1 {
            self.high_start = Some(Instant::now());
        }

        if self.should_yield_to_low_priority() {
            match self.queues.low_priority_rx.try_recv() {
                Ok(mut frame) => {
                    self.hooks.before_send(&mut frame);
                    out.push(frame);
                    self.after_low();
                    *low_closed = false;
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => *low_closed = true,
            }
        }
    }

    fn should_yield_to_low_priority(&self) -> bool {
        let threshold_hit = self.fairness.max_high_before_low > 0
            && self.high_counter >= self.fairness.max_high_before_low;
        let time_hit = self
            .fairness
            .time_slice
            .zip(self.high_start)
            .is_some_and(|(slice, start)| start.elapsed() >= slice);
        threshold_hit || time_hit
    }

    fn after_low(&mut self) { self.reset_high_counter(); }

    fn reset_high_counter(&mut self) {
        self.high_counter = 0;
        self.high_start = None;
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

    /// Await cancellation on the provided shutdown token.
    #[inline]
    async fn wait_shutdown(token: CancellationToken) { token.cancelled_owned().await; }

    /// Receive the next frame from a push queue.
    #[inline]
    async fn recv_push(rx: &mut mpsc::Receiver<F>) -> Option<F> { rx.recv().await }

    /// Poll the current streaming response for the next frame.
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
