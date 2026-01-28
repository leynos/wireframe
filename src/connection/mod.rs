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
        self.multi_packet.install(channel, None);
    }

    /// Set or replace the current multi-packet response channel and stamp correlation identifiers.
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
    /// actor.set_multi_packet_with_correlation(Some(rx), Some(7));
    /// ```
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
        self.multi_packet.install(channel, correlation_id);
    }

    fn clear_multi_packet(&mut self) { self.multi_packet.clear(); }

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
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus operations internally"
    )]
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
}

#[cfg(not(loom))]
pub mod test_support;
