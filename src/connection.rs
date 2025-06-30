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
    hooks::{ConnectionContext, ProtocolHooks},
    push::{FrameLike, PushHandle, PushQueues},
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
///
/// # Examples
///
/// ```no_run
/// use tokio_util::sync::CancellationToken;
/// use wireframe::{connection::ConnectionActor, push::PushQueues};
///
/// let (queues, handle) = PushQueues::<u8>::bounded(8, 8);
/// let shutdown = CancellationToken::new();
/// let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
/// # drop(actor);
/// ```
pub struct ConnectionActor<F, E> {
    high_rx: Option<mpsc::Receiver<F>>,
    low_rx: Option<mpsc::Receiver<F>>,
    response: Option<FrameStream<F, E>>, // current streaming response
    shutdown: CancellationToken,
    hooks: ProtocolHooks<F>,
    ctx: ConnectionContext,
    fairness: FairnessConfig,
    high_counter: usize,
    high_start: Option<Instant>,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike,
{
    /// Create a new `ConnectionActor` from the provided components.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio_util::sync::CancellationToken;
    /// use wireframe::{connection::ConnectionActor, push::PushQueues};
    ///
    /// let (queues, handle) = PushQueues::<u8>::bounded(4, 4);
    /// let token = CancellationToken::new();
    /// let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, token);
    /// # drop(actor);
    /// ```
    #[must_use]
    pub fn new(
        queues: PushQueues<F>,
        handle: PushHandle<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
    ) -> Self {
        Self::with_hooks(queues, handle, response, shutdown, ProtocolHooks::default())
    }

    /// Create a new `ConnectionActor` with custom protocol hooks.
    #[must_use]
    pub fn with_hooks(
        queues: PushQueues<F>,
        handle: PushHandle<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        mut hooks: ProtocolHooks<F>,
    ) -> Self {
        let mut ctx = ConnectionContext;
        hooks.on_connection_setup(handle, &mut ctx);
        Self {
            high_rx: Some(queues.high_priority_rx),
            low_rx: Some(queues.low_priority_rx),
            response,
            shutdown,
            hooks,
            ctx,
            fairness: FairnessConfig::default(),
            high_counter: 0,
            high_start: None,
        }
    }

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

        let mut state = ActorState::new(self.response.is_some());

        while !state.is_done() {
            self.poll_sources(&mut state, out).await?;
        }

        Ok(())
    }

    /// Poll all sources and push available frames into `out`.
    ///
    /// This method polls the shutdown token, high- and low-priority queues,
    /// and the optional response stream. Frames are appended to `out` in the
    /// order they are processed. `ActorState` is updated based on which sources
    /// return `None`.
    async fn poll_sources(
        &mut self,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        let high_available = self.high_rx.is_some();
        let low_available = self.low_rx.is_some();
        let resp_available = self.response.is_some();

        tokio::select! {
            biased;

            () = Self::wait_shutdown(self.shutdown.clone()), if state.is_active() => {
                self.process_shutdown(state);
            }

            res = Self::poll_high(self.high_rx.as_mut()), if high_available => {
                self.process_high(res, state, out);
            }

            res = Self::poll_low(self.low_rx.as_mut()), if low_available => {
                self.process_low(res, state, out);
            }

            // `tokio::select!` is biased so the shutdown branch runs before
            // this one. `process_shutdown` removes the response stream, making
            // `resp_available` false on the next loop iteration. The explicit
            // `!state.is_shutting_down()` check avoids polling the stream after
            // shutdown has begun.
            res = Self::poll_response(self.response.as_mut()), if resp_available && !state.is_shutting_down() => {
                self.process_response(res, state, out)?;
            }
        }

        Ok(())
    }

    /// Begin shutdown once cancellation has been observed.
    fn process_shutdown(&mut self, state: &mut ActorState) {
        state.start_shutdown();
        self.start_shutdown(state);
    }

    /// Handle the result of polling the high-priority queue.
    fn process_high(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        if let Some(mut frame) = res {
            self.hooks.before_send(&mut frame, &mut self.ctx);
            out.push(frame);
            self.after_high(out, state);
        } else {
            self.high_rx = None;
            state.mark_closed();
            self.reset_high_counter();
        }
    }

    /// Handle the result of polling the low-priority queue.
    fn process_low(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        if let Some(mut frame) = res {
            self.hooks.before_send(&mut frame, &mut self.ctx);
            out.push(frame);
            self.after_low();
        } else {
            self.low_rx = None;
            state.mark_closed();
        }
    }

    /// Handle the next frame or error from the streaming response.
    fn process_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        let processed = matches!(res, Some(Ok(_)));
        let is_none = res.is_none();
        self.handle_response(res, state, out)?;
        if processed {
            self.after_low();
        }
        if is_none {
            self.response = None;
        }
        Ok(())
    }

    /// Close all receivers and mark the response stream as closed if present.
    fn start_shutdown(&mut self, state: &mut ActorState) {
        if let Some(rx) = &mut self.high_rx {
            rx.close();
        }
        if let Some(rx) = &mut self.low_rx {
            rx.close();
        }
        if self.response.take().is_some() {
            state.mark_closed();
        }
    }

    /// Update counters and opportunistically drain the low-priority queue.
    fn after_high(&mut self, out: &mut Vec<F>, state: &mut ActorState) {
        self.high_counter += 1;
        if self.high_counter == 1 {
            self.high_start = Some(Instant::now());
        }

        if self.should_yield_to_low_priority()
            && let Some(rx) = &mut self.low_rx
        {
            match rx.try_recv() {
                Ok(mut frame) => {
                    self.hooks.before_send(&mut frame, &mut self.ctx);
                    out.push(frame);
                    self.after_low();
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.low_rx = None;
                    state.mark_closed();
                }
            }
        }
    }

    /// Determine if processing should yield to the low-priority queue.
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

    /// Reset counters after processing a low-priority frame.
    fn after_low(&mut self) { self.reset_high_counter(); }

    /// Clear the burst counter and associated timestamp.
    fn reset_high_counter(&mut self) {
        self.high_counter = 0;
        self.high_start = None;
    }

    /// Push a frame from the response stream into `out` or handle completion.
    fn handle_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        match res {
            Some(Ok(mut frame)) => {
                self.hooks.before_send(&mut frame, &mut self.ctx);
                out.push(frame);
            }
            Some(Err(e)) => return Err(e),
            None => {
                state.mark_closed();
                self.hooks.on_command_end(&mut self.ctx);
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

    /// Future for polling the high-priority queue if present.
    #[allow(clippy::manual_async_fn)]
    fn poll_high(
        rx: Option<&mut mpsc::Receiver<F>>,
    ) -> impl std::future::Future<Output = Option<F>> + '_ {
        async move {
            if let Some(rx) = rx {
                Self::recv_push(rx).await
            } else {
                None
            }
        }
    }

    /// Future for polling the low-priority queue if present.
    #[allow(clippy::manual_async_fn)]
    fn poll_low(
        rx: Option<&mut mpsc::Receiver<F>>,
    ) -> impl std::future::Future<Output = Option<F>> + '_ {
        async move {
            if let Some(rx) = rx {
                Self::recv_push(rx).await
            } else {
                None
            }
        }
    }

    /// Future for polling the response stream if present.
    #[allow(clippy::manual_async_fn)]
    fn poll_response(
        stream: Option<&mut FrameStream<F, E>>,
    ) -> impl std::future::Future<Output = Option<Result<F, WireframeError<E>>>> + '_ {
        async move {
            if let Some(s) = stream {
                s.next().await
            } else {
                None
            }
        }
    }
}

/// Internal run state for the connection actor.
enum RunState {
    /// All sources are open and frames are still being processed.
    Active,
    /// A shutdown request has been observed and queues are being closed.
    ShuttingDown,
    /// All sources have completed and the actor can exit.
    Finished,
}

/// Tracks progress through the actor lifecycle.
struct ActorState {
    run_state: RunState,
    closed_sources: usize,
    total_sources: usize,
}

impl ActorState {
    /// Create a new `ActorState`.
    ///
    /// `has_response` indicates whether a streaming response is currently
    /// attached.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use wireframe::connection::ActorState;
    ///
    /// let state = ActorState::new(true);
    /// assert!(state.is_active());
    /// ```
    fn new(has_response: bool) -> Self {
        Self {
            run_state: RunState::Active,
            // The shutdown token is considered closed until cancellation
            // occurs, matching previous behaviour where draining sources
            // without explicit shutdown terminates the actor.
            closed_sources: 1,
            total_sources: 3 + usize::from(has_response),
        }
    }

    /// Mark a source as closed and update the run state if all are closed.
    fn mark_closed(&mut self) {
        self.closed_sources += 1;
        if self.closed_sources == self.total_sources {
            self.run_state = RunState::Finished;
        }
    }

    /// Transition to `ShuttingDown` if currently active.
    fn start_shutdown(&mut self) {
        if matches!(self.run_state, RunState::Active) {
            self.run_state = RunState::ShuttingDown;
        }
    }

    /// Returns `true` while the actor is actively processing sources.
    fn is_active(&self) -> bool { matches!(self.run_state, RunState::Active) }

    /// Returns `true` once shutdown has begun.
    fn is_shutting_down(&self) -> bool { matches!(self.run_state, RunState::ShuttingDown) }

    /// Returns `true` when all sources have finished.
    fn is_done(&self) -> bool { matches!(self.run_state, RunState::Finished) }
}
