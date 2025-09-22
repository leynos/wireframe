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
    multi_packet: Option<mpsc::Receiver<F>>,
    shutdown: CancellationToken,
    counter: Option<ActiveConnection>,
    hooks: ProtocolHooks<F, E>,
    ctx: ConnectionContext,
    fairness: FairnessTracker,
    connection_id: Option<ConnectionId>,
    peer_addr: Option<SocketAddr>,
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
    pub fn set_response(&mut self, stream: Option<FrameStream<F, E>>) { self.response = stream; }

    /// Set or replace the current multi-packet response channel.
    pub fn set_multi_packet(&mut self, channel: Option<mpsc::Receiver<F>>) {
        debug_assert!(
            self.response.is_none(),
            "ConnectionActor invariant violated: cannot set multi_packet while a response stream \
             is active"
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
        if let Some(frame) = res {
            self.process_frame_common(frame, out);
            self.after_high(out, state);
        } else {
            Self::handle_closed_receiver(&mut self.high_rx, state);
            self.fairness.reset();
        }
    }

    /// Process a queue-backed source with shared low-priority semantics.
    fn process_queue<CloseFn>(
        &mut self,
        res: Option<F>,
        state: &mut ActorState,
        out: &mut Vec<F>,
        on_close: CloseFn,
    ) where
        CloseFn: FnOnce(&mut Self, &mut ActorState, &mut Vec<F>),
    {
        if let Some(frame) = res {
            self.process_frame_common(frame, out);
            self.after_low();
        } else {
            on_close(self, state, out);
        }
    }

    /// Handle the result of polling the low-priority queue.
    fn process_low(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        self.process_queue(res, state, out, |s, st, _out| {
            Self::handle_closed_receiver(&mut s.low_rx, st);
        });
    }

    /// Handle frames drained from the multi-packet channel.
    fn process_multi_packet(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        self.process_queue(res, state, out, |s, st, out| {
            s.multi_packet = None;
            st.mark_closed();
            let mut emitted_end = false;
            if let Some(mut frame) = s.hooks.stream_end_frame(&mut s.ctx) {
                s.hooks.before_send(&mut frame, &mut s.ctx);
                out.push(frame);
                crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
                emitted_end = true;
            }
            s.hooks.on_command_end(&mut s.ctx);
            if emitted_end {
                s.after_low();
            }
        });
    }

    /// Common logic for processing frames from push queues.
    fn process_frame_common(&mut self, frame: F, out: &mut Vec<F>) {
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

        if self.fairness.should_yield_to_low_priority() {
            // 1) Try low-priority queue first.
            if let Some(res) = self.low_rx.as_mut().map(mpsc::Receiver::try_recv) {
                match res {
                    Ok(mut frame) => {
                        self.hooks.before_send(&mut frame, &mut self.ctx);
                        out.push(frame);
                        self.after_low();
                        return;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        Self::handle_closed_receiver(&mut self.low_rx, state);
                    }
                }
            }
            // 2) Opportunistically yield to the multi-packet channel as the same class.
            if let Some(res) = self.multi_packet.as_mut().map(mpsc::Receiver::try_recv) {
                match res {
                    Ok(mut frame) => {
                        self.hooks.before_send(&mut frame, &mut self.ctx);
                        out.push(frame);
                        self.after_low();
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        self.multi_packet = None;
                        state.mark_closed();
                    }
                }
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
            Some(Ok(mut frame)) => {
                self.hooks.before_send(&mut frame, &mut self.ctx);
                out.push(frame);
                crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
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
                if let Some(mut frame) = self.hooks.stream_end_frame(&mut self.ctx) {
                    self.hooks.before_send(&mut frame, &mut self.ctx);
                    out.push(frame);
                    crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
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
