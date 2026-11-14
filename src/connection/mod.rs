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
mod grace;
mod init;
mod interruption;
mod multi_packet;
mod observation;
mod output;
mod peer_close;
mod polling;
mod pushes;
mod response;
mod setters;
mod shutdown;
mod sink;
mod state;
#[cfg(all(test, not(loom)))]
mod tests;
mod writes;

use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
};

pub use channels::ConnectionChannels;
pub(crate) use counter::ActiveConnection;
#[cfg(loom)]
pub use counter::LoomConnectionGuard;
pub use counter::active_connection_count;
pub(crate) use drive::{PEER_CLOSED_RESPONSE_GRACE, ResponseOutcome};
use grace::{GraceTimer, TokioGraceTimer};
use init::ActorInit;
pub(crate) use init::UntrackedActorInit;
use log::info;
pub(crate) use observation::UnobservedCause;
use output::ActiveOutput;
pub(crate) use sink::{FrameSink, ProgressSignal, PushSink};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
// `WriteBound` is only named outside `writes` by its own unit tests: production
// callers ask the outcome's predicates instead, so the re-export follows the
// test module's gate rather than warning as unused in a library build.
#[cfg(all(test, not(loom)))]
pub(crate) use writes::WriteBound;
pub(crate) use writes::{WriteOutcome, write_before_bound};

pub use crate::fairness::FairnessConfig;
use crate::{
    NoProtocolError,
    app::Packet,
    correlation::CorrelatableFrame,
    fairness::FairnessTracker,
    fragment::Fragmenter,
    hooks::{ConnectionContext, ProtocolHooks},
    push::{FrameLike, PushHandle, PushQueues},
    response::FrameStream,
    session::ConnectionId,
};

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
/// `F` is the frame type of handler responses and streamed or multi-packet
/// output. `P` is the frame type taken from the push queues; it defaults to `F`
/// so a standalone actor treats queued frames exactly like responses. The
/// server selects a distinct `P` because pushes arrive already encoded for the
/// wire while responses are serialized from application envelopes.
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
pub struct ConnectionActor<F, E = NoProtocolError, P = F> {
    /// Receiver for urgent frames, polled before all other data sources.
    high_rx: Option<mpsc::Receiver<P>>,
    /// Receiver for best-effort frames, serviced subject to fairness limits.
    low_rx: Option<mpsc::Receiver<P>>,
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
    /// Composition applied to frames taken from the push queues.
    ///
    /// A standalone actor composes pushes like response frames. A server-owned
    /// actor defers composition to its transport sink, which runs the
    /// `before_send` hook on the already-encoded frame.
    composer: pushes::PushComposer<F, E, P>,
    /// Identifier used to correlate actor diagnostics with connection setup.
    connection_id: Option<ConnectionId>,
    /// Peer address retained for diagnostics when the socket exposes it.
    peer_addr: Option<SocketAddr>,
    /// Whether a completion hook is waiting for the final frame to be written.
    command_end_pending: bool,
    /// Whether the response currently being driven reported a protocol error.
    response_failed: bool,
    /// Whether a shutdown signal abandons a response still being driven.
    ///
    /// The direct actor API keeps shutdown precedence; a server connection opts
    /// out so ADR-014's drain delivers a response its peer is still reading.
    abandon_response_on_shutdown: bool,
    /// Token cancelled when the peer closes its input half.
    peer_closed: CancellationToken,
    /// Idle grace applied to a response once the peer can no longer be observed.
    ///
    /// Production connections keep [`drive::PEER_CLOSED_RESPONSE_GRACE`]; tests
    /// shorten it so a paused clock can exercise cancellation deterministically.
    peer_closed_grace: std::time::Duration,
    /// Deadline source that enforces [`Self::peer_closed_grace`].
    ///
    /// Production uses the ambient Tokio clock. A test installs a manually
    /// fired deadline, so it proves cancellation happens exactly when the grace
    /// expires without depending on the wall clock.
    grace_timer: Arc<dyn GraceTimer>,
    /// Correlation identifier of the response currently being driven.
    response_correlation: Option<u64>,
    /// Why direct peer observation stopped for the current dispatch.
    ///
    /// The inbound loop fills this before cancelling [`Self::peer_closed`], so
    /// the cancellation warning can name the cause.
    unobserved_cause: Option<Arc<OnceLock<UnobservedCause>>>,
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
        // The handle is cloned for setup only: retaining a second strong handle
        // would keep the queues open and hide lane closure from the actor.
        let handle = channels.handle.clone();
        let mut actor = Self::build(
            ActorInit {
                channels,
                response,
                shutdown,
                hooks,
                ctx: ConnectionContext::default(),
                composer: pushes::PushComposer::Inline(Self::compose_inline_push),
            },
            Some(ActiveConnection::new()),
        );
        actor.run_connection_setup(handle);
        actor
    }

    /// Run the protocol setup callback with this actor's push handle.
    fn run_connection_setup(&mut self, handle: PushHandle<F>) {
        self.hooks.on_connection_setup(handle, &mut self.ctx);
    }

    /// Compose a queued frame exactly like a response frame.
    fn compose_inline_push(actor: &mut Self, frame: F, pushes: &mut Vec<F>) {
        actor.process_frame_with_hooks_and_metrics(frame, pushes);
    }
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Create an actor without registering it in the active-connection gauge.
    ///
    /// The server creates one untracked actor per accepted connection because
    /// the accept loop already holds an [`ActiveConnection`] guard for the
    /// whole connection lifetime. That avoids counting a connection twice. The
    /// server has already published the push handle and run the protocol setup
    /// callback, so the actor must not repeat either.
    #[must_use]
    pub(crate) fn new_untracked(init: UntrackedActorInit<F, E, P>) -> Self {
        Self::build(
            ActorInit {
                channels: init.channels,
                response: init.response,
                shutdown: init.shutdown,
                hooks: init.hooks,
                ctx: init.ctx,
                composer: pushes::PushComposer::Deferred,
            },
            None,
        )
    }

    /// Shared constructor used by the tracked and untracked entry points.
    fn build(init: ActorInit<F, E, P>, counter: Option<ActiveConnection>) -> Self {
        let ActorInit {
            channels,
            response,
            shutdown,
            hooks,
            ctx,
            composer,
        } = init;
        let ConnectionChannels { queues, .. } = channels;
        let connection_id = ctx.connection_id();
        let active_output = response.map_or_else(|| ActiveOutput::None, ActiveOutput::Response);
        let actor = Self {
            high_rx: Some(queues.high_priority_rx),
            low_rx: Some(queues.low_priority_rx),
            active_output,
            shutdown,
            counter,
            hooks,
            ctx,
            fairness: FairnessTracker::new(FairnessConfig::default()),
            fragmenter: None,
            composer,
            connection_id,
            peer_addr: None,
            command_end_pending: false,
            response_failed: false,
            abandon_response_on_shutdown: true,
            peer_closed: CancellationToken::new(),
            peer_closed_grace: PEER_CLOSED_RESPONSE_GRACE,
            grace_timer: Arc::new(TokioGraceTimer),
            response_correlation: None,
            unobserved_cause: None,
        };
        // The server owns the connection lifecycle in its accept loop, so only
        // a tracked actor logs an opening that the gauge count accounts for.
        if actor.counter.is_some() {
            info!(
                "connection opened: wireframe_active_connections={}, id={:?}, peer={:?}",
                counter::current_count(),
                actor.connection_id,
                actor.peer_addr
            );
        }
        actor
    }
}

#[cfg(all(not(loom), any(test, feature = "test-support")))]
pub mod test_support;
