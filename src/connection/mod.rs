//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

mod dispatch;
mod drain;
mod event;
mod frame;
mod multi_packet;
mod polling;
mod response;
mod shutdown;
mod state;

use std::{
    fmt,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use event::Event;
use log::info;
use multi_packet::MultiPacketContext;
use state::ActorState;
use tokio::{sync::mpsc, time::Duration};
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
    app::Packet,
    correlation::CorrelatableFrame,
    fairness::FairnessTracker,
    fragment::{FragmentationConfig, Fragmenter},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{FrameLike, PushHandle, PushQueues},
    response::{FrameStream, WireframeError},
    session::ConnectionId,
};

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

/// Bundles push queues with their shared handle for actor construction.
pub struct ConnectionChannels<F> {
    /// Receivers for high- and low-priority frames consumed by the actor.
    pub queues: PushQueues<F>,
    /// Handle cloned by producers to enqueue frames into the shared queues.
    pub handle: PushHandle<F>,
}

impl<F> ConnectionChannels<F> {
    /// Create a new bundle of push queues and their associated handle.
    #[must_use]
    pub fn new(queues: PushQueues<F>, handle: PushHandle<F>) -> Self { Self { queues, handle } }
}

/// Error returned when attempting to set an active output source while
/// another source is already active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStateError {
    /// A multi-packet channel is currently active and must be cleared before
    /// setting a response stream.
    MultiPacketActive,
    /// A response stream is currently active and must be cleared before
    /// setting a multi-packet channel.
    ResponseActive,
}

impl fmt::Display for ConnectionStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultiPacketActive => write!(
                f,
                "cannot set response while a multi-packet channel is active"
            ),
            Self::ResponseActive => write!(
                f,
                "cannot set multi-packet channel while a response stream is active"
            ),
        }
    }
}

impl std::error::Error for ConnectionStateError {}

/// Active output source for the connection actor.
///
/// At most one output source can be active at a time. This enum makes the
/// mutual exclusion compile-time enforced rather than runtime-asserted.
enum ActiveOutput<F, E> {
    /// No output source is active.
    None,
    /// A streaming response is active.
    Response(FrameStream<F, E>),
    /// A multi-packet channel is active.
    MultiPacket(MultiPacketContext<F>),
}

/// Result of shutting down an active output source.
struct ShutdownResult {
    /// Correlation ID of the multi-packet context, if any.
    correlation_id: Option<u64>,
    /// Whether the source should be marked as closed in `ActorState`.
    source_closed: bool,
    /// Whether `on_command_end` hook should be called.
    call_on_command_end: bool,
}

/// Result of closing an active multi-packet channel.
struct MultiPacketCloseResult {
    /// Correlation ID of the multi-packet context, if any.
    correlation_id: Option<u64>,
}

impl<F, E> ActiveOutput<F, E> {
    /// Returns `true` if a streaming response is active.
    fn is_response(&self) -> bool { matches!(self, Self::Response(_)) }

    /// Returns `true` if a multi-packet channel is active.
    fn is_multi_packet(&self) -> bool { matches!(self, Self::MultiPacket(_)) }

    /// Returns a mutable reference to the multi-packet context if active.
    fn multi_packet_mut(&mut self) -> Option<&mut MultiPacketContext<F>> {
        match self {
            Self::MultiPacket(ctx) => Some(ctx),
            _ => Option::None,
        }
    }

    /// Clears the response stream, leaving `None` in its place.
    fn clear_response(&mut self) {
        if matches!(self, Self::Response(_)) {
            *self = Self::None;
        }
    }

    /// Perform shutdown cleanup and return the result.
    ///
    /// This takes ownership of the active output, closes any receivers, and
    /// returns metadata needed by the caller to complete shutdown handling.
    fn shutdown(&mut self) -> ShutdownResult {
        match std::mem::replace(self, Self::None) {
            Self::MultiPacket(mut ctx) => {
                let correlation_id = ctx.correlation_id();
                let source_closed = if let Some(rx) = ctx.channel_mut() {
                    rx.close();
                    true
                } else {
                    false
                };
                ShutdownResult {
                    correlation_id,
                    source_closed,
                    call_on_command_end: true,
                }
            }
            Self::Response(_) => ShutdownResult {
                correlation_id: None,
                source_closed: true,
                call_on_command_end: false,
            },
            Self::None => ShutdownResult {
                correlation_id: None,
                source_closed: false,
                call_on_command_end: false,
            },
        }
    }

    /// Begin closing a multi-packet channel.
    ///
    /// This closes the receiver and returns the correlation ID for logging,
    /// but does NOT clear the context yet. The caller must call `clear()` after
    /// emitting any terminator frames that need correlation IDs applied.
    fn close_multi_packet(&mut self) -> MultiPacketCloseResult {
        let correlation_id = self.multi_packet_mut().and_then(|ctx| ctx.correlation_id());
        if let Self::MultiPacket(ctx) = self
            && let Some(rx) = ctx.channel_mut()
        {
            rx.close();
        }
        MultiPacketCloseResult { correlation_id }
    }

    /// Clear the active output to `None`.
    fn clear(&mut self) { *self = Self::None; }
}

/// Availability flags for event sources polled by the connection actor.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Availability flags are a natural fit for booleans; no state machine needed"
)]
#[derive(Clone, Copy)]
struct EventAvailability {
    high: bool,
    low: bool,
    multi_packet: bool,
    response: bool,
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
    /// Active output source: either a streaming response or a multi-packet channel.
    ///
    /// At most one output source can be active at a time. The multi-packet channel
    /// is drained after low-priority frames to preserve fairness with queued sources.
    /// The actor emits the protocol terminator when the sender closes the channel.
    active_output: ActiveOutput<F, E>,
    shutdown: CancellationToken,
    counter: Option<ActiveConnection>,
    hooks: ProtocolHooks<F, E>,
    ctx: ConnectionContext,
    fairness: FairnessTracker,
    fragmenter: Option<Arc<Fragmenter>>,
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
    F: FrameLike + CorrelatableFrame + Packet,
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
            ConnectionChannels::new(queues, handle),
            response,
            shutdown,
            ProtocolHooks::<F, E>::default(),
        )
    }

    /// Create a new `ConnectionActor` with custom protocol hooks.
    #[must_use]
    pub fn with_hooks(
        channels: ConnectionChannels<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        hooks: ProtocolHooks<F, E>,
    ) -> Self {
        let ConnectionChannels { queues, handle } = channels;
        let ctx = ConnectionContext;
        let counter = ActiveConnection::new();
        let active_output = match response {
            Some(stream) => ActiveOutput::Response(stream),
            None => ActiveOutput::None,
        };
        let mut actor = Self {
            high_rx: Some(queues.high_priority_rx),
            low_rx: Some(queues.low_priority_rx),
            active_output,
            shutdown,
            counter: Some(counter),
            hooks,
            ctx,
            fairness: FairnessTracker::new(FairnessConfig::default()),
            fragmenter: None,
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

    /// Enable transparent fragmentation for outbound frames.
    ///
    /// When configured, frames that exceed `fragment_payload_cap` are split
    /// into multiple fragments carrying a standard fragment header inside the
    /// payload. Callers continue to enqueue complete frames; fragmentation
    /// occurs just before hooks and metrics are applied.
    pub fn enable_fragmentation(&mut self, config: FragmentationConfig)
    where
        F: Packet,
    {
        self.fragmenter = Some(Arc::new(Fragmenter::new(config.fragment_payload_cap)));
    }

    /// Set or replace the current streaming response.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionStateError::MultiPacketActive`] if a multi-packet
    /// channel is currently active.
    pub fn set_response(
        &mut self,
        stream: Option<FrameStream<F, E>>,
    ) -> Result<(), ConnectionStateError> {
        if self.active_output.is_multi_packet() {
            return Err(ConnectionStateError::MultiPacketActive);
        }
        self.active_output = match stream {
            Some(s) => ActiveOutput::Response(s),
            None => ActiveOutput::None,
        };
        Ok(())
    }

    /// Set or replace the current multi-packet response channel.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionStateError::ResponseActive`] if a response stream is
    /// currently active.
    pub fn set_multi_packet(
        &mut self,
        channel: Option<mpsc::Receiver<F>>,
    ) -> Result<(), ConnectionStateError> {
        self.set_multi_packet_with_correlation(channel, None)
    }

    /// Set or replace the current multi-packet response channel and stamp correlation identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionStateError::ResponseActive`] if a response stream is
    /// currently active.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tokio::sync::mpsc;
    /// # use tokio_util::sync::CancellationToken;
    /// # use wireframe::{ConnectionActor, push::PushQueues};
    /// # let (queues, handle) = PushQueues::<u8>::builder()
    /// #     .high_capacity(1)
    /// #     .low_capacity(1)
    /// #     .build()
    /// #     .expect("failed to build PushQueues");
    /// # let shutdown = CancellationToken::new();
    /// # let mut actor = ConnectionActor::new(queues, handle, None, shutdown);
    /// # let (_tx, rx) = mpsc::channel(4);
    /// actor.set_multi_packet_with_correlation(Some(rx), Some(7))?;
    /// # Ok::<(), wireframe::connection::ConnectionStateError>(())
    /// ```
    pub fn set_multi_packet_with_correlation(
        &mut self,
        channel: Option<mpsc::Receiver<F>>,
        correlation_id: Option<u64>,
    ) -> Result<(), ConnectionStateError> {
        if self.active_output.is_response() {
            return Err(ConnectionStateError::ResponseActive);
        }
        self.active_output = match channel {
            Some(rx) => {
                let mut ctx = MultiPacketContext::new();
                ctx.install(Some(rx), correlation_id);
                ActiveOutput::MultiPacket(ctx)
            }
            None => ActiveOutput::None,
        };
        Ok(())
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

        let mut state = ActorState::new(
            self.active_output.is_response(),
            self.active_output.is_multi_packet(),
        );

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

    /// Compute which event sources are currently available for polling.
    fn compute_availability(&self, state: &ActorState) -> EventAvailability {
        EventAvailability {
            high: self.high_rx.is_some(),
            low: self.low_rx.is_some(),
            multi_packet: self.active_output.is_multi_packet() && !state.is_shutting_down(),
            response: self.active_output.is_response() && !state.is_shutting_down(),
        }
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
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus operations internally"
    )]
    async fn next_event(&mut self, state: &ActorState) -> Event<F, E> {
        let avail = self.compute_availability(state);

        // Extract mutable references before the select! to satisfy the borrow
        // checker. Only one of these can be Some due to the ActiveOutput enum
        // invariant.
        let (multi_rx, response_stream) = match &mut self.active_output {
            ActiveOutput::MultiPacket(ctx) => (ctx.channel_mut(), None),
            ActiveOutput::Response(stream) => (None, Some(stream)),
            ActiveOutput::None => (None, None),
        };

        tokio::select! {
            biased;

            () = Self::wait_shutdown(self.shutdown.clone()), if state.is_active() => Event::Shutdown,
            res = Self::poll_queue(self.high_rx.as_mut()), if avail.high => Event::High(res),
            res = Self::poll_queue(self.low_rx.as_mut()), if avail.low => Event::Low(res),
            res = Self::poll_queue(multi_rx), if avail.multi_packet => Event::MultiPacket(res),
            res = Self::poll_response(response_stream), if avail.response => Event::Response(res),
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
}

#[cfg(not(loom))]
pub mod test_support;
