//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

use std::{
    future::Future,
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
};

use futures::StreamExt;
use log::{info, warn};
use tokio::{
    sync::mpsc::{self, error::TryRecvError},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// Global gauge tracking active connections.
static ACTIVE_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// RAII guard incrementing [`ACTIVE_CONNECTIONS`] on creation and
/// decrementing it on drop.
struct ActiveConnection;

impl ActiveConnection {
    fn new() -> Self {
        ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
        crate::metrics::inc_connections();
        Self
    }
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
        crate::metrics::dec_connections();
    }
}

/// Return the current number of active connections.
#[must_use]
pub fn active_connection_count() -> u64 { ACTIVE_CONNECTIONS.load(Ordering::Relaxed) }

use crate::{
    fairness::FairnessTracker,
    hooks::{ConnectionContext, ProtocolHooks},
    push::{FrameLike, PushHandle, PushQueues},
    response::{FrameStream, WireframeError},
    session::ConnectionId,
};

/// Events returned by [`next_event`].
///
/// Only `Debug` is derived because `WireframeError<E>` does not implement
/// `Clone` or `PartialEq`.
#[derive(Debug)]
enum Event<F, E> {
    Shutdown,
    High(Option<F>),
    Low(Option<F>),
    /// Frames drained from the multi-packet response channel.
    /// Frames are forwarded in channel order after low-priority queues to
    /// preserve fairness and reuse the existing back-pressure.
    /// The actor emits the protocol terminator when the sender closes the
    /// channel so downstream observers see end-of-stream signalling.
    MultiPacket(Option<F>),
    Response(Option<Result<F, WireframeError<E>>>),
    Idle,
}

/// Configuration controlling fairness when draining push queues.
#[derive(Clone, Copy, Debug)]
pub struct FairnessConfig {
    /// Number of consecutive high-priority frames to process before
    /// checking the low-priority queue.
    ///
    /// A zero value disables the counter and relies solely on
    /// `time_slice` for fairness, preserving strict high-priority
    /// ordering otherwise.
    pub max_high_before_low: usize,
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
/// let (queues, handle) = PushQueues::<u8>::builder()
///     .high_capacity(8)
///     .low_capacity(8)
///     .build()
///     .expect("failed to build PushQueues");
/// let shutdown = CancellationToken::new();
/// let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
/// # drop(actor);
/// ```
pub struct ConnectionActor<F, E> {
    high_rx: Option<mpsc::Receiver<F>>,
    low_rx: Option<mpsc::Receiver<F>>,
    response: Option<FrameStream<F, E>>, // current streaming response
    /// Optional multi-packet channel drained after low-priority frames.
    /// This preserves fairness with queued sources.
    /// The actor emits the protocol terminator when the sender closes the channel.
    multi_packet: Option<mpsc::Receiver<F>>,
    shutdown: CancellationToken,
    counter: Option<ActiveConnection>,
    hooks: ProtocolHooks<F, E>,
    ctx: ConnectionContext,
    fairness: FairnessTracker,
    connection_id: Option<ConnectionId>,
    peer_addr: Option<SocketAddr>,
}

/// Context for drain operations containing mutable references to output and actor state.
struct DrainContext<'a, F> {
    out: &'a mut Vec<F>,
    state: &'a mut ActorState,
}

/// Queue variants processed by the connection actor.
#[derive(Clone, Copy)]
enum QueueKind {
    High,
    Low,
    Multi,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike,
    E: std::fmt::Debug,
{
    /// Create a new `ConnectionActor` from the provided components.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tokio_util::sync::CancellationToken;
    /// use wireframe::{connection::ConnectionActor, push::PushQueues};
    ///
    /// let (queues, handle) = PushQueues::<u8>::builder()
    ///     .high_capacity(4)
    ///     .low_capacity(4)
    ///     .build()
    ///     .expect("failed to build PushQueues");
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
        Self::with_hooks(
            queues,
            handle,
            response,
            shutdown,
            ProtocolHooks::<F, E>::default(),
        )
    }

    /// Create a new `ConnectionActor` with custom protocol hooks.
    #[must_use]
    pub fn with_hooks(
        queues: PushQueues<F>,
        handle: PushHandle<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        hooks: ProtocolHooks<F, E>,
    ) -> Self {
        let ctx = ConnectionContext;
        let counter = ActiveConnection::new();
        let mut actor = Self {
            high_rx: Some(queues.high_priority_rx),
            low_rx: Some(queues.low_priority_rx),
            response,
            multi_packet: None,
            shutdown,
            counter: Some(counter),
            hooks,
            ctx,
            fairness: FairnessTracker::new(FairnessConfig::default()),
            connection_id: None,
            peer_addr: None,
        };
        let current = ACTIVE_CONNECTIONS.load(Ordering::Relaxed);
        info!(
            "connection opened: wireframe_active_connections={}, id={:?}, peer={:?}",
            current, actor.connection_id, actor.peer_addr
        );
        actor.hooks.on_connection_setup(handle, &mut actor.ctx);
        actor
    }

    /// Replace the fairness configuration.
    pub fn set_fairness(&mut self, fairness: FairnessConfig) { self.fairness.set_config(fairness); }

    /// Set or replace the current streaming response.
    pub fn set_response(&mut self, stream: Option<FrameStream<F, E>>) {
        debug_assert!(
            self.multi_packet.is_none(),
            concat!(
                "ConnectionActor invariant violated: cannot set response while a ",
                "multi_packet channel is active"
            ),
        );
        self.response = stream;
    }

    /// Set or replace the current multi-packet response channel.
    pub fn set_multi_packet(&mut self, channel: Option<mpsc::Receiver<F>>) {
        debug_assert!(
            self.response.is_none(),
            concat!(
                "ConnectionActor invariant violated: cannot set multi_packet while a ",
                "response stream is active"
            ),
        );
        self.multi_packet = channel;
    }

    /// Get a clone of the shutdown token used by the actor.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken { self.shutdown.clone() }

    /// Drive the actor until all sources are exhausted or shutdown is triggered.
    ///
    /// Frames are appended to `out` in the order they are processed.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the response stream yields an I/O error.
    pub async fn run(&mut self, out: &mut Vec<F>) -> Result<(), WireframeError<E>> {
        // Spans removed in favour of standardised log facade.
        // If cancellation has already been requested, exit immediately. Nothing
        // will be drained and any streaming response is abandoned. This mirrors
        // a hard shutdown and is required for the tests.
        if self.shutdown.is_cancelled() {
            info!(
                "connection aborted before start: id={:?}, peer={:?}",
                self.connection_id, self.peer_addr
            );
            let _ = self.counter.take();
            return Ok(());
        }

        debug_assert!(
            usize::from(self.response.is_some()) + usize::from(self.multi_packet.is_some()) <= 1,
            "ConnectionActor invariant violated: at most one of response or multi_packet may be \
             active"
        );
        let mut state = ActorState::new(self.response.is_some(), self.multi_packet.is_some());

        while !state.is_done() {
            self.poll_sources(&mut state, out).await?;
        }
        info!(
            "connection closed: id={:?}, peer={:?}",
            self.connection_id, self.peer_addr
        );
        let _ = self.counter.take();
        Ok(())
    }

    /// Await the next ready event using biased priority ordering.
    ///
    /// Shutdown is observed first, followed by high-priority pushes, then
    /// low-priority pushes, multi-packet channels, and finally the response
    /// stream. This mirrors the
    /// original behaviour and matches the design documentation. The final
    /// `else` branch prevents `tokio::select!` from panicking if all guards are
    /// false.
    ///
    /// The `strict_priority_order` and `shutdown_signal_precedence` tests
    /// assert that this ordering is preserved across refactors.
    async fn next_event(&mut self, state: &ActorState) -> Event<F, E> {
        let high_available = self.high_rx.is_some();
        let low_available = self.low_rx.is_some();
        let multi_available = self.multi_packet.is_some() && !state.is_shutting_down();
        let resp_available = self.response.is_some() && !state.is_shutting_down();

        tokio::select! {
            biased;

            () = Self::await_shutdown(self.shutdown.clone()), if state.is_active() => Event::Shutdown,

            res = Self::poll_queue(self.high_rx.as_mut()), if high_available => Event::High(res),

            res = Self::poll_queue(self.low_rx.as_mut()), if low_available => Event::Low(res),

            res = Self::poll_queue(self.multi_packet.as_mut()), if multi_available => Event::MultiPacket(res),

            res = Self::poll_response(self.response.as_mut()), if resp_available => Event::Response(res),

            else => Event::Idle,
        }
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
        let event = self.next_event(state).await;
        self.dispatch_event(event, state, out)
    }

    /// Dispatch the given event to the appropriate handler.
    fn dispatch_event(
        &mut self,
        event: Event<F, E>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        match event {
            Event::Shutdown => self.process_shutdown(state),
            Event::High(res) => self.process_high(res, state, out),
            Event::Low(res) => self.process_low(res, state, out),
            Event::MultiPacket(res) => self.process_multi_packet(res, state, out),
            Event::Response(res) => self.process_response(res, state, out)?,
            Event::Idle => {}
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
        self.process_queue(QueueKind::High, res, DrainContext { out, state });
    }

    /// Process a queue-backed source with shared low-priority semantics.
    fn process_queue(&mut self, kind: QueueKind, res: Option<F>, ctx: DrainContext<'_, F>) {
        let DrainContext { out, state } = ctx;
        match res {
            Some(frame) => {
                self.process_frame_with_hooks_and_metrics(frame, out);
                match kind {
                    QueueKind::High => self.after_high(out, state),
                    QueueKind::Low | QueueKind::Multi => self.after_low(),
                }
            }
            None => match kind {
                QueueKind::High => {
                    Self::handle_closed_receiver(&mut self.high_rx, state);
                    self.fairness.reset();
                }
                QueueKind::Low => {
                    Self::handle_closed_receiver(&mut self.low_rx, state);
                }
                QueueKind::Multi => {
                    self.handle_multi_packet_closed(state, out);
                }
            },
        }
    }

    /// Handle the result of polling the low-priority queue.
    fn process_low(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        self.process_queue(QueueKind::Low, res, DrainContext { out, state });
    }

    /// Handle frames drained from the multi-packet channel.
    fn process_multi_packet(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        self.process_queue(QueueKind::Multi, res, DrainContext { out, state });
    }

    /// Handle a closed multi-packet channel by emitting the protocol terminator and notifying
    /// hooks.
    fn handle_multi_packet_closed(&mut self, state: &mut ActorState, out: &mut Vec<F>) {
        self.multi_packet = None;
        state.mark_closed();
        if let Some(frame) = self.hooks.stream_end_frame(&mut self.ctx) {
            self.process_frame_with_hooks_and_metrics(frame, out);
            self.after_low();
        }
        self.hooks.on_command_end(&mut self.ctx);
    }

    /// Apply protocol hooks and increment metrics before emitting a frame.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// actor.process_frame_with_hooks_and_metrics(frame, &mut out);
    /// ```
    fn process_frame_with_hooks_and_metrics(&mut self, frame: F, out: &mut Vec<F>) {
        let mut frame = frame;
        self.hooks.before_send(&mut frame, &mut self.ctx);
        out.push(frame);
        crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
    }

    /// Common logic for handling closed receivers.
    fn handle_closed_receiver(receiver: &mut Option<mpsc::Receiver<F>>, state: &mut ActorState) {
        *receiver = None;
        state.mark_closed();
    }

    /// Handle the next frame or error from the streaming response.
    fn process_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        let is_none = res.is_none();
        let produced = self.handle_response(res, state, out)?;
        if produced {
            self.after_low();
        }
        if is_none {
            self.response = None;
        }
        Ok(())
    }

    /// Close all receivers and mark streaming sources as closed if present.
    fn start_shutdown(&mut self, state: &mut ActorState) {
        if let Some(rx) = &mut self.high_rx {
            rx.close();
        }
        if let Some(rx) = &mut self.low_rx {
            rx.close();
        }
        if let Some(mut rx) = self.multi_packet.take() {
            rx.close();
            state.mark_closed();
        }
        if self.response.take().is_some() {
            state.mark_closed();
        }
    }

    /// Update counters and opportunistically drain the low-priority and multi-packet queues.
    fn after_high(&mut self, out: &mut Vec<F>, state: &mut ActorState) {
        self.fairness.record_high_priority();

        if !self.fairness.should_yield_to_low_priority() {
            return;
        }

        let mut low_rx = self.low_rx.take();
        let low_handler = self.create_close_handler(QueueKind::Low);
        if self.try_opportunistic_drain(
            &mut low_rx,
            DrainContext {
                out: &mut *out,
                state: &mut *state,
            },
            low_handler,
        ) {
            self.low_rx = low_rx;
            return;
        }
        self.low_rx = low_rx;

        let mut multi_rx = self.multi_packet.take();
        let multi_handler = self.create_close_handler(QueueKind::Multi);
        let _ = self.try_opportunistic_drain(
            &mut multi_rx,
            DrainContext {
                out: &mut *out,
                state: &mut *state,
            },
            multi_handler,
        );
        self.multi_packet = multi_rx;
    }

    /// Try to opportunistically drain a queue-backed source when fairness allows.
    ///
    /// Returns `true` when a frame is forwarded to `out`.
    fn try_opportunistic_drain<OnClose>(
        &mut self,
        rx: &mut Option<mpsc::Receiver<F>>,
        ctx: DrainContext<'_, F>,
        on_close: OnClose,
    ) -> bool
    where
        OnClose: FnOnce(&mut Self, DrainContext<'_, F>),
    {
        let res = match rx {
            Some(receiver) => receiver.try_recv(),
            None => return false,
        };

        match res {
            Ok(frame) => {
                self.process_frame_with_hooks_and_metrics(frame, ctx.out);
                self.after_low();
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                *rx = None;
                on_close(self, ctx);
                false
            }
        }
    }

    /// Create a closure that handles receiver shutdown for the given queue kind.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let handler = actor.create_close_handler(QueueKind::Low);
    /// ```
    #[expect(
        clippy::unused_self,
        reason = "Receiver retained for future stateful close handlers"
    )]
    fn create_close_handler(
        &mut self,
        kind: QueueKind,
    ) -> impl FnOnce(&mut Self, DrainContext<'_, F>) + use<F, E> {
        move |actor, ctx| match kind {
            QueueKind::High => {
                let DrainContext { state, .. } = ctx;
                Self::handle_closed_receiver(&mut actor.high_rx, state);
                actor.fairness.reset();
            }
            QueueKind::Low => {
                let DrainContext { state, .. } = ctx;
                Self::handle_closed_receiver(&mut actor.low_rx, state);
            }
            QueueKind::Multi => {
                let DrainContext { out, state } = ctx;
                actor.handle_multi_packet_closed(state, out);
            }
        }
    }

    /// Reset counters after processing a low-priority frame.
    fn after_low(&mut self) { self.fairness.reset(); }

    /// Push a frame from the response stream into `out` or handle completion.
    ///
    /// Protocol errors are passed to `handle_error` and do not terminate the
    /// actor. I/O errors propagate to the caller.
    ///
    /// Returns `true` if a frame was appended to `out`.
    fn handle_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<bool, WireframeError<E>> {
        let mut produced = false;
        match res {
            Some(Ok(frame)) => {
                self.process_frame_with_hooks_and_metrics(frame, out);
                produced = true;
            }
            Some(Err(WireframeError::Protocol(e))) => {
                warn!("protocol error: error={e:?}");
                self.hooks.handle_error(e, &mut self.ctx);
                state.mark_closed();
                // Stop polling the response after a protocol error to avoid
                // double-closing and duplicate `on_command_end` signalling.
                self.response = None;
                self.hooks.on_command_end(&mut self.ctx);
                crate::metrics::inc_handler_errors();
            }
            Some(Err(e)) => return Err(e),
            None => {
                state.mark_closed();
                if let Some(frame) = self.hooks.stream_end_frame(&mut self.ctx) {
                    self.process_frame_with_hooks_and_metrics(frame, out);
                    produced = true;
                }
                self.hooks.on_command_end(&mut self.ctx);
            }
        }

        Ok(produced)
    }

    /// Await cancellation on the provided shutdown token.
    #[inline]
    async fn wait_shutdown(token: CancellationToken) { token.cancelled_owned().await; }

    /// Receive the next frame from a push queue.
    #[inline]
    async fn recv_push(rx: &mut mpsc::Receiver<F>) -> Option<F> { rx.recv().await }

    /// Poll `f` if `opt` is `Some`, returning `None` otherwise.
    #[expect(
        clippy::manual_async_fn,
        reason = "Generic lifetime requires explicit async move"
    )]
    fn poll_optional<'a, T, Fut, R>(
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
    async fn await_shutdown(token: CancellationToken) { Self::wait_shutdown(token).await; }

    /// Poll whichever receiver is provided, returning `None` when absent.
    ///
    /// Multi-packet channels reuse this helper so they share back-pressure with queued frames.
    async fn poll_queue(rx: Option<&mut mpsc::Receiver<F>>) -> Option<F> {
        Self::poll_optional(rx, Self::recv_push).await
    }

    /// Poll the streaming response.
    async fn poll_response(
        resp: Option<&mut FrameStream<F, E>>,
    ) -> Option<Result<F, WireframeError<E>>> {
        Self::poll_optional(resp, |s| s.next()).await
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
    /// `has_multi_packet` signals that a channel-backed response is active.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use wireframe::connection::ActorState;
    ///
    /// let state = ActorState::new(true, false);
    /// assert!(state.is_active());
    /// ```
    fn new(has_response: bool, has_multi_packet: bool) -> Self {
        Self {
            run_state: RunState::Active,
            // The shutdown token is considered closed until cancellation
            // occurs, matching previous behaviour where draining sources
            // without explicit shutdown terminates the actor.
            closed_sources: 1,
            // total_sources counts all possible sources that can keep the actor alive:
            // - 3 for the base sources (main, shutdown, and drain)
            // - +1 if a response is expected (has_response)
            // - +1 if multi-packet handling is enabled (has_multi_packet)
            total_sources: 3 + usize::from(has_response) + usize::from(has_multi_packet),
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

#[cfg(all(test, not(loom)))]
mod tests {
    //! Tests covering connection actor queue helpers and multi-packet handling.

    use std::{
        cell::Cell,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use rstest::rstest;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{hooks::ConnectionContext, push::PushQueues};

    /// Build a connection actor with the supplied hooks.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let actor = create_test_actor_with_hooks(ProtocolHooks::default());
    /// ```
    fn create_test_actor_with_hooks(hooks: ProtocolHooks<u8, ()>) -> ConnectionActor<u8, ()> {
        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(4)
            .low_capacity(4)
            .build()
            .expect("failed to build PushQueues");
        ConnectionActor::with_hooks(queues, handle, None, CancellationToken::new(), hooks)
    }

    /// Prepare an actor, state, and output buffer for multi-packet scenarios.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let (mut actor, mut state, mut out) = setup_multi_packet_test();
    /// ```
    fn setup_multi_packet_test() -> (ConnectionActor<u8, ()>, ActorState, Vec<u8>) {
        let actor = create_test_actor_with_hooks(ProtocolHooks::<u8, ()>::default());
        let state = ActorState::new(false, true);
        let out = Vec::new();
        (actor, state, out)
    }

    /// Assert that processed frames and hook counters match expectations.
    ///
    /// The counter arguments represent the absolute difference between the
    /// observed and expected hook invocations.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_frame_processed(&[1], &[1], 0, 0);
    /// ```
    fn assert_frame_processed(out: &[u8], expected: &[u8], before_delta: usize, end_delta: usize) {
        assert_eq!(out, expected, "frames should match expected output");
        assert_eq!(before_delta, 0, "unexpected before_send hook count");
        assert_eq!(end_delta, 0, "unexpected on_command_end hook count");
    }

    /// Assert actor state flags for easier readability in tests.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_state_flags(&state, true, false, false);
    /// ```
    fn assert_state_flags(state: &ActorState, active: bool, shutting_down: bool, done: bool) {
        assert_eq!(state.is_active(), active, "active flag mismatch");
        assert_eq!(
            state.is_shutting_down(),
            shutting_down,
            "shutdown flag mismatch"
        );
        assert_eq!(state.is_done(), done, "completion flag mismatch");
    }

    #[test]
    fn process_multi_packet_forwards_frame() {
        let before_calls = Arc::new(AtomicUsize::new(0));
        let before_clone = before_calls.clone();
        let hooks = ProtocolHooks {
            before_send: Some(Box::new(
                move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                    *frame += 1;
                },
            )),
            ..ProtocolHooks::<u8, ()>::default()
        };

        let mut actor = create_test_actor_with_hooks(hooks);
        let mut state = ActorState::new(false, false);
        let mut out = Vec::new();

        actor.process_multi_packet(Some(5), &mut state, &mut out);

        let before_actual = before_calls.load(Ordering::SeqCst);
        assert_frame_processed(&out, &[6], before_actual.abs_diff(1), 0);
        assert_state_flags(&state, true, false, false);
    }

    #[test]
    fn process_multi_packet_none_emits_end_frame() {
        let before_calls = Arc::new(AtomicUsize::new(0));
        let before_clone = before_calls.clone();
        let end_calls = Arc::new(AtomicUsize::new(0));
        let end_clone = end_calls.clone();
        let hooks = ProtocolHooks {
            before_send: Some(Box::new(
                move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                    *frame += 2;
                },
            )),
            on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
                end_clone.fetch_add(1, Ordering::SeqCst);
            })),
            stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| Some(9))),
            ..ProtocolHooks::<u8, ()>::default()
        };

        let (mut actor, mut state, mut out) = setup_multi_packet_test();
        actor.hooks = hooks;
        let (_tx, rx) = mpsc::channel(1);
        actor.multi_packet = Some(rx);

        actor.process_multi_packet(None, &mut state, &mut out);

        let before_actual = before_calls.load(Ordering::SeqCst);
        let end_actual = end_calls.load(Ordering::SeqCst);
        assert_frame_processed(
            &out,
            &[11],
            before_actual.abs_diff(1),
            end_actual.abs_diff(1),
        );
        assert_state_flags(&state, true, false, false);
    }

    #[rstest(
        terminator, expected_output, expected_before,
        case::with_terminator(Some(5), vec![6], 1),
        case::without_terminator(None, Vec::new(), 0),
    )]
    fn handle_multi_packet_closed_behaviour(
        terminator: Option<u8>,
        expected_output: Vec<u8>,
        expected_before: usize,
    ) {
        let before_calls = Arc::new(AtomicUsize::new(0));
        let before_clone = before_calls.clone();
        let end_calls = Arc::new(AtomicUsize::new(0));
        let end_clone = end_calls.clone();
        let hooks = ProtocolHooks {
            before_send: Some(Box::new(
                move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                    *frame += 1;
                },
            )),
            on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
                end_clone.fetch_add(1, Ordering::SeqCst);
            })),
            stream_end: Some(Box::new(move |_ctx: &mut ConnectionContext| terminator)),
            ..ProtocolHooks::<u8, ()>::default()
        };

        let mut actor = create_test_actor_with_hooks(hooks);
        let (_tx, rx) = mpsc::channel(1);
        actor.multi_packet = Some(rx);
        let mut state = ActorState::new(false, true);
        let mut out = Vec::new();

        actor.handle_multi_packet_closed(&mut state, &mut out);

        let before_actual = before_calls.load(Ordering::SeqCst);
        let end_actual = end_calls.load(Ordering::SeqCst);
        assert_frame_processed(
            &out,
            &expected_output,
            before_actual.abs_diff(expected_before),
            end_actual.abs_diff(1),
        );
        assert!(
            actor.multi_packet.is_none(),
            "multi-packet channel should be cleared"
        );
        assert_state_flags(&state, true, false, false);
    }

    #[test]
    fn try_opportunistic_drain_forwards_frame() {
        let before_calls = Arc::new(AtomicUsize::new(0));
        let before_clone = before_calls.clone();
        let hooks = ProtocolHooks {
            before_send: Some(Box::new(
                move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                    *frame += 1;
                },
            )),
            ..ProtocolHooks::<u8, ()>::default()
        };
        let mut actor = create_test_actor_with_hooks(hooks);
        let mut state = ActorState::new(false, false);
        let mut out = Vec::new();

        let (tx, rx) = mpsc::channel(1);
        tx.try_send(9).expect("send frame");
        drop(tx);

        let mut queue = Some(rx);
        let drained = actor.try_opportunistic_drain(
            &mut queue,
            DrainContext {
                out: &mut out,
                state: &mut state,
            },
            |_actor, _ctx| unreachable!("on_close should not be called"),
        );

        let before_actual = before_calls.load(Ordering::SeqCst);
        assert!(drained, "queue should report a drained frame");
        assert!(queue.is_some(), "queue remains available");
        assert_frame_processed(&out, &[10], before_actual.abs_diff(1), 0);
    }

    #[test]
    fn try_opportunistic_drain_returns_false_when_empty() {
        let mut actor = create_test_actor_with_hooks(ProtocolHooks::<u8, ()>::default());
        let mut state = ActorState::new(false, false);
        let mut out = Vec::new();
        let (_tx, rx) = mpsc::channel(1);
        let mut queue = Some(rx);

        let drained = actor.try_opportunistic_drain(
            &mut queue,
            DrainContext {
                out: &mut out,
                state: &mut state,
            },
            |_actor, _ctx| unreachable!("on_close should not be called"),
        );

        assert!(!drained, "no frame should be drained");
        assert!(queue.is_some(), "queue should remain available");
        assert_frame_processed(&out, &[], 0, 0);
    }

    #[test]
    fn try_opportunistic_drain_handles_disconnect() {
        let mut actor = create_test_actor_with_hooks(ProtocolHooks::<u8, ()>::default());
        let mut state = ActorState::new(false, false);
        let mut out = Vec::new();
        let (tx, rx) = mpsc::channel(1);
        drop(tx);
        let mut queue = Some(rx);
        let closed = Cell::new(false);

        let drained = actor.try_opportunistic_drain(
            &mut queue,
            DrainContext {
                out: &mut out,
                state: &mut state,
            },
            |_actor, ctx| {
                closed.set(true);
                let DrainContext { state, .. } = ctx;
                state.mark_closed();
            },
        );

        assert!(!drained, "disconnect should not produce a frame");
        assert!(queue.is_none(), "queue should be cleared after disconnect");
        assert!(closed.get(), "on_close should be invoked for disconnect");
        assert_frame_processed(&out, &[], 0, 0);
    }

    #[tokio::test]
    async fn poll_queue_reads_frame() {
        let (tx, mut rx) = mpsc::channel(1);
        tx.send(42).await.expect("send frame");

        let value = ConnectionActor::<u8, ()>::poll_queue(Some(&mut rx)).await;

        assert_eq!(value, Some(42));
    }

    #[tokio::test]
    async fn poll_queue_returns_none_for_absent_receiver() {
        let value = ConnectionActor::<u8, ()>::poll_queue(None).await;
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn poll_queue_returns_none_after_close() {
        let (tx, mut rx) = mpsc::channel(1);
        drop(tx);

        let value = ConnectionActor::<u8, ()>::poll_queue(Some(&mut rx)).await;

        assert!(value.is_none());
    }

    #[rstest(
        has_multi,
        expected_marks,
        case::with_multi(true, 3),
        case::without_multi(false, 2)
    )]
    fn actor_state_tracks_sources(has_multi: bool, expected_marks: usize) {
        let mut state = ActorState::new(false, has_multi);
        assert_state_flags(&state, true, false, false);

        for _ in 0..expected_marks.saturating_sub(1) {
            state.mark_closed();
            assert_state_flags(&state, true, false, false);
        }

        state.mark_closed();
        assert_state_flags(&state, false, false, true);
    }
}
