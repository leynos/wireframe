//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

mod channels;
mod counter;
mod dispatch;
mod drain;
mod drive;
mod event;
mod frame;
mod multi_packet;
mod output;
mod polling;
mod response;
mod shutdown;
mod sink;
mod state;
#[cfg(all(test, not(loom)))]
mod tests;

use std::net::SocketAddr;

pub use channels::ConnectionChannels;
pub(crate) use counter::ActiveConnection;
#[cfg(loom)]
pub use counter::LoomConnectionGuard;
pub use counter::active_connection_count;
pub(crate) use drive::ResponseOutcome;
use log::info;
use multi_packet::MultiPacketContext;
use output::ActiveOutput;
pub(crate) use sink::FrameSink;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub use crate::fairness::FairnessConfig;
use crate::{
    NoProtocolError,
    app::Packet,
    correlation::CorrelatableFrame,
    fairness::FairnessTracker,
    fragment::{FragmentationConfig, Fragmenter},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{FrameLike, PushHandle, PushQueues},
    response::FrameStream,
    session::ConnectionId,
};

/// Construction inputs shared by the tracked and untracked actor
/// constructors.
struct ActorInit<F, E> {
    /// Push queues and the handle that keeps them open.
    channels: ConnectionChannels<F>,
    /// Optional response stream installed at construction time.
    response: Option<FrameStream<F, E>>,
    /// Cancellation token observed by the actor loop.
    shutdown: CancellationToken,
    /// Protocol hooks applied to outbound frames.
    hooks: ProtocolHooks<F, E>,
}

impl<F, E> ActorInit<F, E> {
    /// Bundle the construction inputs both constructors accept.
    const fn new(
        channels: ConnectionChannels<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        hooks: ProtocolHooks<F, E>,
    ) -> Self {
        Self {
            channels,
            response,
            shutdown,
            hooks,
        }
    }
}

/// Error returned when attempting to set an active output source while
/// another source is already active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ConnectionStateError {
    /// A multi-packet channel is currently active and must be cleared before
    /// setting a response stream.
    #[error("cannot set response while a multi-packet channel is active")]
    MultiPacketActive,
    /// A response stream is currently active and must be cleared before
    /// setting a multi-packet channel.
    #[error("cannot set multi-packet channel while a response stream is active")]
    ResponseActive,
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
/// let mut actor: ConnectionActor<_> = ConnectionActor::new(queues, handle, None, shutdown);
/// # drop(actor);
/// ```
pub struct ConnectionActor<F, E = NoProtocolError> {
    /// Receiver for urgent frames, polled before all other data sources.
    high_rx: Option<mpsc::Receiver<F>>,
    /// Receiver for best-effort frames, serviced subject to fairness limits.
    low_rx: Option<mpsc::Receiver<F>>,
    /// Active output source: either a streaming response or a multi-packet channel.
    ///
    /// At most one output source can be active at a time. The multi-packet channel
    /// is drained after low-priority frames to preserve fairness with queued sources.
    /// The actor emits the protocol terminator after the sender closes the channel only when
    /// `stream_end_frame` returns `Some`.
    active_output: ActiveOutput<F, E>,
    /// Cancellation signal shared by the supervisor and this actor.
    shutdown: CancellationToken,
    /// RAII registration keeping the global active-connection gauge accurate.
    counter: Option<ActiveConnection>,
    /// Protocol callbacks invoked around frame and stream lifecycle events.
    hooks: ProtocolHooks<F, E>,
    /// Per-connection context passed to protocol callbacks.
    ctx: ConnectionContext,
    /// Scheduler state preventing high-priority traffic from starving low traffic.
    fairness: FairnessTracker,
    /// Optional fragmenter applied immediately before outbound emission.
    fragmenter: Option<Fragmenter>,
    /// Identifier used to correlate actor diagnostics with connection setup.
    connection_id: Option<ConnectionId>,
    /// Peer address retained for diagnostics when the socket exposes it.
    peer_addr: Option<SocketAddr>,
    /// Whether a completion hook is waiting for the final frame to be written.
    command_end_pending: bool,
    /// Whether the response currently being driven reported a protocol error.
    response_failed: bool,
    /// Token cancelled when the peer closes its input half.
    peer_closed: CancellationToken,
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
    /// let mut actor: ConnectionActor<_> = ConnectionActor::new(queues, handle, None, token);
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
        let connection = ActiveConnection::new();
        Self::build(
            ActorInit::new(channels, response, shutdown, hooks),
            Some(connection),
        )
    }

    /// Create an actor without registering it in the active-connection gauge.
    ///
    /// The server creates one untracked actor per accepted connection because
    /// the accept loop already holds an [`ActiveConnection`] guard for the
    /// whole connection lifetime. That avoids counting a connection twice.
    #[must_use]
    pub(crate) fn new_untracked(
        channels: ConnectionChannels<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        hooks: ProtocolHooks<F, E>,
    ) -> Self {
        Self::build(ActorInit::new(channels, response, shutdown, hooks), None)
    }

    /// Shared constructor used by the tracked and untracked entry points.
    fn build(init: ActorInit<F, E>, counter: Option<ActiveConnection>) -> Self {
        let ActorInit {
            channels,
            response,
            shutdown,
            hooks,
        } = init;
        let ConnectionChannels { queues, handle } = channels;
        let ctx = ConnectionContext;
        let active_output = response.map_or_else(|| ActiveOutput::None, ActiveOutput::Response);
        let mut actor = Self {
            high_rx: Some(queues.high_priority_rx),
            low_rx: Some(queues.low_priority_rx),
            active_output,
            shutdown,
            counter,
            hooks,
            ctx,
            fairness: FairnessTracker::new(FairnessConfig::default()),
            fragmenter: None,
            connection_id: None,
            peer_addr: None,
            command_end_pending: false,
            response_failed: false,
            peer_closed: CancellationToken::new(),
        };
        // The server builds an untracked actor per connection and owns the
        // connection lifecycle in its accept loop. Logging an opening there
        // would be unbalanced because `install_response`/`drive_response`
        // never log a close, and the active-connection count would not include
        // that connection.
        if actor.counter.is_some() {
            info!(
                "connection opened: wireframe_active_connections={}, id={:?}, peer={:?}",
                counter::current_count(),
                actor.connection_id,
                actor.peer_addr
            );
        }
        actor.hooks.on_connection_setup(handle, &mut actor.ctx);
        actor
    }

    /// Share the token that reports the peer closing its input half.
    ///
    /// The inbound loop cancels the token once its read half reports EOF. A
    /// response that is still streaming is then driven under a bounded idle
    /// grace, because a peer that has vanished will never read the remaining
    /// frames or send another request.
    pub(crate) fn set_peer_closed(&mut self, token: CancellationToken) { self.peer_closed = token; }

    /// Replace the fairness configuration.
    pub const fn set_fairness(&mut self, fairness: FairnessConfig) {
        self.fairness.set_config(fairness);
    }

    /// Enable transparent fragmentation for outbound frames.
    ///
    /// When configured, frames that exceed `fragment_payload_cap` are split
    /// into multiple fragments carrying a standard fragment header inside the
    /// payload. Callers continue to enqueue complete frames; fragmentation
    /// occurs just before hooks and metrics are applied.
    pub const fn enable_fragmentation(&mut self, config: FragmentationConfig)
    where
        F: Packet,
    {
        self.fragmenter = Some(Fragmenter::new(config.fragment_payload_cap));
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
        self.active_output = stream.map_or_else(|| ActiveOutput::None, ActiveOutput::Response);
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
    /// # use wireframe::{connection::ConnectionActor, push::PushQueues};
    /// # let (queues, handle) = PushQueues::<u8>::builder()
    /// #     .high_capacity(1)
    /// #     .low_capacity(1)
    /// #     .build()
    /// #     .expect("failed to build PushQueues");
    /// # let shutdown = CancellationToken::new();
    /// # let mut actor: ConnectionActor<u8> = ConnectionActor::new(queues, handle, None, shutdown);
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
        self.active_output = channel.map_or_else(
            || ActiveOutput::None,
            |rx| {
                let mut ctx = MultiPacketContext::new();
                ctx.install(Some(rx), correlation_id);
                ActiveOutput::MultiPacket(ctx)
            },
        );
        Ok(())
    }

    /// Replace the low-priority queue used for tests.
    pub fn set_low_queue(&mut self, queue: Option<mpsc::Receiver<F>>) { self.low_rx = queue; }

    /// Get a clone of the shutdown token used by the actor.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken { self.shutdown.clone() }
}

#[cfg(all(not(loom), any(test, feature = "test-support")))]
pub mod test_support;
