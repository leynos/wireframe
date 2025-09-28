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
    correlation::CorrelatableFrame,
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
    multi_packet: MultiPacketContext<F>,
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

/// Multi-packet correlation stamping state.
///
/// Tracks the active receiver and how frames should be stamped before emission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MultiPacketStamp {
    /// Stamping is disabled because no multi-packet channel is active.
    Disabled,
    /// Stamping is enabled and frames are stamped with the provided identifier.
    Enabled(Option<u64>),
}

/// Multi-packet channel state tracking the active receiver and stamping config.
struct MultiPacketContext<F> {
    channel: Option<mpsc::Receiver<F>>,
    stamp: MultiPacketStamp,
}

impl<F> MultiPacketContext<F> {
    const fn new() -> Self {
        Self {
            channel: None,
            stamp: MultiPacketStamp::Disabled,
        }
    }

    fn install(&mut self, channel: Option<mpsc::Receiver<F>>, stamp: MultiPacketStamp) {
        debug_assert_eq!(
            channel.is_some(),
            matches!(stamp, MultiPacketStamp::Enabled(_)),
            "multi-packet correlation must be provided when the channel is active",
        );
        self.channel = channel;
        self.stamp = stamp;
    }

    fn clear(&mut self) {
        self.channel = None;
        self.stamp = MultiPacketStamp::Disabled;
    }

    fn channel_mut(&mut self) -> Option<&mut mpsc::Receiver<F>> { self.channel.as_mut() }

    fn take_channel(&mut self) -> Option<mpsc::Receiver<F>> { self.channel.take() }

    fn stamp(&self) -> MultiPacketStamp { self.stamp }

    fn is_active(&self) -> bool { self.channel.is_some() }
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
    F: FrameLike + CorrelatableFrame,
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
            multi_packet: MultiPacketContext::new(),
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
            !self.multi_packet.is_active(),
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
        let stamp = if channel.is_some() {
            MultiPacketStamp::Enabled(None)
        } else {
            MultiPacketStamp::Disabled
        };
        self.multi_packet.install(channel, stamp);
    }

    /// Set or replace the current multi-packet response channel and stamp correlation identifiers.
    pub fn set_multi_packet_with_correlation(
        &mut self,
        channel: Option<mpsc::Receiver<F>>,
        correlation_id: Option<u64>,
    ) {
        debug_assert!(
            self.response.is_none(),
            concat!(
                "ConnectionActor invariant violated: cannot set multi_packet while a ",
                "response stream is active"
            ),
        );
        let stamp = if channel.is_some() {
            MultiPacketStamp::Enabled(correlation_id)
        } else {
            MultiPacketStamp::Disabled
        };
        self.multi_packet.install(channel, stamp);
    }

    fn clear_multi_packet(&mut self) { self.multi_packet.clear(); }

    fn apply_multi_packet_correlation(&mut self, frame: &mut F) {
        match self.multi_packet.stamp() {
            MultiPacketStamp::Enabled(Some(expected)) => {
                frame.set_correlation_id(Some(expected));
                debug_assert_eq!(
                    frame.correlation_id(),
                    Some(expected),
                    "multi-packet frame correlation mismatch: expected={:?}, got={:?}",
                    Some(expected),
                    frame.correlation_id(),
                );
            }
            MultiPacketStamp::Enabled(None) => {
                frame.set_correlation_id(None);
                debug_assert!(
                    frame.correlation_id().is_none(),
                    "multi-packet frame correlation unexpectedly present: got={:?}",
                    frame.correlation_id(),
                );
            }
            MultiPacketStamp::Disabled => {
                unreachable!("multi-packet correlation invoked without configuration");
            }
        }
    }

    /// Replace the low-priority queue used for tests.
    pub fn set_low_queue(&mut self, queue: Option<mpsc::Receiver<F>>) { self.low_rx = queue; }

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
            usize::from(self.response.is_some()) + usize::from(self.multi_packet.is_active()) <= 1,
            "ConnectionActor invariant violated: at most one of response or multi_packet may be \
             active"
        );
        let mut state = ActorState::new(self.response.is_some(), self.multi_packet.is_active());

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
        let multi_available = self.multi_packet.is_active() && !state.is_shutting_down();
        let resp_available = self.response.is_some() && !state.is_shutting_down();

        tokio::select! {
            biased;

            () = Self::await_shutdown(self.shutdown.clone()), if state.is_active() => Event::Shutdown,
            res = Self::poll_queue(self.high_rx.as_mut()), if high_available => Event::High(res),
            res = Self::poll_queue(self.low_rx.as_mut()), if low_available => Event::Low(res),
            res = Self::poll_queue(self.multi_packet.channel_mut()), if multi_available => Event::MultiPacket(res),
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
                match kind {
                    QueueKind::Multi
                        if matches!(self.multi_packet.stamp(), MultiPacketStamp::Enabled(_)) =>
                    {
                        self.emit_multi_packet_frame(frame, out);
                    }
                    _ => {
                        self.process_frame_with_hooks_and_metrics(frame, out);
                    }
                }
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

    fn emit_multi_packet_frame(&mut self, frame: F, out: &mut Vec<F>) {
        let mut frame = frame;
        self.apply_multi_packet_correlation(&mut frame);
        self.process_frame_with_hooks_and_metrics(frame, out);
    }

    /// Handle a closed multi-packet channel by emitting the protocol terminator and notifying
    /// hooks.
    fn handle_multi_packet_closed(&mut self, state: &mut ActorState, out: &mut Vec<F>) {
        let rx = self.multi_packet.take_channel();
        self.handle_multi_packet_closed_with(rx, state, out);
    }

    /// Handle multi-packet closure using an already-extracted receiver option.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// actor.handle_multi_packet_closed_with(None, &mut state, &mut out);
    /// ```
    fn handle_multi_packet_closed_with(
        &mut self,
        rx: Option<mpsc::Receiver<F>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) {
        if let Some(mut receiver) = rx {
            receiver.close();
        }
        state.mark_closed();
        if let Some(frame) = self.hooks.stream_end_frame(&mut self.ctx) {
            self.emit_multi_packet_frame(frame, out);
            self.after_low();
        }
        self.clear_multi_packet();
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
        if let Some(mut rx) = self.multi_packet.take_channel() {
            rx.close();
            state.mark_closed();
            self.clear_multi_packet();
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

        if self.try_opportunistic_drain(
            QueueKind::Low,
            DrainContext {
                out: &mut *out,
                state: &mut *state,
            },
        ) {
            return;
        }

        let _ = self.try_opportunistic_drain(
            QueueKind::Multi,
            DrainContext {
                out: &mut *out,
                state: &mut *state,
            },
        );
    }

    /// Try to opportunistically drain a queue-backed source when fairness allows.
    ///
    /// Returns `true` when a frame is forwarded to `out`.
    fn try_opportunistic_drain(&mut self, kind: QueueKind, ctx: DrainContext<'_, F>) -> bool {
        let DrainContext { out, state } = ctx;
        match kind {
            QueueKind::High => unreachable!(concat!(
                "try_opportunistic_drain(High) is unsupported; ",
                "High is handled by biased polling",
            )),
            QueueKind::Low => {
                let res = match self.low_rx.as_mut() {
                    Some(receiver) => receiver.try_recv(),
                    None => return false,
                };

                match res {
                    Ok(frame) => {
                        self.process_frame_with_hooks_and_metrics(frame, out);
                        self.after_low();
                        true
                    }
                    Err(TryRecvError::Empty) => false,
                    Err(TryRecvError::Disconnected) => {
                        Self::handle_closed_receiver(&mut self.low_rx, state);
                        false
                    }
                }
            }
            QueueKind::Multi => {
                let result = match self.multi_packet.channel_mut() {
                    Some(rx) => rx.try_recv(),
                    None => return false,
                };

                match result {
                    Ok(frame) => {
                        self.emit_multi_packet_frame(frame, out);
                        self.after_low();
                        true
                    }
                    Err(TryRecvError::Empty) => false,
                    Err(TryRecvError::Disconnected) => {
                        let rx = self.multi_packet.take_channel();
                        self.handle_multi_packet_closed_with(rx, state, out);
                        false
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
            // total_sources counts all sources that keep the actor alive:
            // - 3 for the baseline sources (main loop, shutdown token, and queue drains)
            // - +1 if a streaming response is active (has_response)
            // - +1 if multi-packet handling is enabled (has_multi_packet)
            total_sources: 3 + usize::from(has_response) + usize::from(has_multi_packet),
        }
    }

    /// Mark a source as closed and update the run state if all are closed.
    fn mark_closed(&mut self) {
        self.closed_sources += 1;
        if self.closed_sources >= self.total_sources {
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

#[cfg(not(loom))]
pub mod test_support;
