//! Per-connection output actor used by the inbound frame loop.
//!
//! Application responses are installed on an untracked [`ConnectionActor`]
//! instead of being written directly. The actor owns the biased selection
//! rules that would interleave pushed frames with response streams, so a
//! handler that returns `Response::Stream` or `Response::MultiPacket` is
//! driven to completion through those rules. The internal push queues are
//! structural for now: the server neither exposes the handle nor installs
//! server-initiated pushes on this actor.
//!
//! The actor deliberately receives no fragmenter and no `before_send` hook.
//! Fragmentation, codec wrapping, and protocol hooks for application frames
//! all happen in the transport sink owned by the caller, which keeps a single
//! ordering for serialization, hooks, and write metrics.

use std::{
    io,
    net::SocketAddr,
    sync::{Arc, OnceLock},
};

use tokio_util::sync::CancellationToken;

use super::recovery::RecoveryCoordinator;
use crate::{
    app::Envelope,
    codec::{FrameCodec, RecoveryConfig, RecoveryPolicyHook},
    connection::{
        ConnectionActor,
        ConnectionChannels,
        PEER_CLOSED_RESPONSE_GRACE,
        UnobservedCause,
    },
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    push::{PushHandle, PushQueues},
};

/// Capacity of each internal push queue.
///
/// The server never exposes the handle, so these queues are only a structural
/// requirement of the actor. A small bound is sufficient and keeps the
/// per-connection allocation predictable.
const INTERNAL_PUSH_CAPACITY: usize = 8;

/// Per-connection actor plus a handle that keeps its push queues open.
///
/// The handle is intentionally not part of the public connection API: server
/// pushes are not wired through this actor yet, but the senders must outlive
/// the actor so the queues do not close underneath it.
pub(super) struct ConnectionResponseActor {
    /// Actor driven by the inbound frame loop.
    pub(super) actor: ConnectionActor<Envelope, ()>,
    /// Unexposed handle keeping the high- and low-priority queues open.
    _push_handle: PushHandle<Envelope>,
}

/// Cancels the per-connection token when stream processing exits.
///
/// The token is independent of server shutdown: ending a response must not be
/// tied to the lifetime of the whole server.
pub(super) struct CancellationGuard(CancellationToken);

impl CancellationGuard {
    /// Wrap a token so it is cancelled when this guard is dropped.
    pub(super) const fn new(token: CancellationToken) -> Self { Self(token) }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) { self.0.cancel(); }
}

/// Build the untracked actor that drives responses for one connection.
///
/// Returns the actor and the queue handle that keeps its push receivers open.
/// Callers must hold the handle for as long as the actor is used.
///
/// The peer-observation token is deliberately not supplied here. The inbound
/// loop installs a fresh per-dispatch token on every frame, because buffering
/// a pipelined request makes the peer unobservable only for the response that
/// is in flight; a connection-wide token would leak that state into later
/// responses.
pub(super) fn build_connection_actor<F>(
    protocol: Option<&Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    shutdown: CancellationToken,
) -> io::Result<(ConnectionResponseActor, CancellationGuard)>
where
    F: FrameCodec,
{
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(INTERNAL_PUSH_CAPACITY)
        .low_capacity(INTERNAL_PUSH_CAPACITY)
        .build()
        .map_err(|error| io::Error::other(format!("failed to build actor queues: {error}")))?;
    let hooks = protocol.map_or_else(ProtocolHooks::default, |registered| {
        actor_hooks::<F>(registered)
    });
    let mut actor = ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle.clone()),
        None,
        shutdown.clone(),
        hooks,
    );
    // The connection boundary owns the idle-grace policy; the actor only needs
    // a bound, not an opinion about how long a vanished peer may hold a
    // response open.
    actor.set_peer_closed_grace(PEER_CLOSED_RESPONSE_GRACE);
    // ADR-014 keeps a connection draining a readable peer across `stop()`, so
    // shutdown must not abandon the response drive here. The stop token still
    // bounds every write, which is what prevents a peer that stopped reading
    // from pinning the drain.
    actor.set_abandon_response_on_shutdown(false);
    Ok((
        ConnectionResponseActor {
            actor,
            _push_handle: handle,
        },
        CancellationGuard::new(shutdown),
    ))
}

/// Install the per-dispatch peer signal and return the token and cause cell.
///
/// A fresh child token per dispatch inherits a permanent EOF, but cancelling it
/// leaves the connection-wide token untouched, so one buffered request cannot
/// make every later response run under the idle grace. The cause cell is filled
/// when the loop stops polling the read half, so the actor's cancellation
/// warning can name the cause.
pub(super) fn observe_peer(
    actor: &mut ConnectionActor<Envelope, ()>,
    peer_eof: &CancellationToken,
) -> (CancellationToken, Arc<OnceLock<UnobservedCause>>) {
    let unobserved = peer_eof.child_token();
    let cause = Arc::new(OnceLock::new());
    actor.set_peer_observation(unobserved.clone(), Arc::clone(&cause));
    (unobserved, cause)
}

/// Adapt a protocol into the hooks the actor needs for response frames.
///
/// Only the error callback is forwarded. Terminator emission and completion
/// run in the inbound handler after the actor has finished driving the
/// response: the protocol returns a complete codec frame, and writing that
/// frame unchanged is the only way to preserve metadata a custom codec adds
/// outside the payload. `before_send` stays unset because the transport sink
/// runs it after serialization. The protocol's setup and EOF callbacks are not
/// invoked on the prepared path at all, so they are deliberately absent here.
fn actor_hooks<F>(
    protocol: &Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>,
) -> ProtocolHooks<Envelope, ()>
where
    F: FrameCodec,
{
    let handle_error = {
        let error_protocol = Arc::clone(protocol);
        Box::new(move |error: (), ctx: &mut ConnectionContext| {
            error_protocol.handle_error(error, ctx);
        })
    };

    ProtocolHooks {
        handle_error: Some(handle_error),
        ..ProtocolHooks::default()
    }
}

/// Per-connection handles that outlive any single inbound frame.
///
/// The read loop keeps these for the whole connection: the token bounding it,
/// the token recording a peer that will send nothing further, the actor that
/// drives responses, and the coordinator that decides what a decoder failure
/// means. Building them together names each policy once, in one place.
pub(super) struct ConnectionRuntime<'a> {
    /// Cancelled when this connection must stop reading.
    pub(super) shutdown: CancellationToken,
    /// Cancelled once a clean EOF makes further peer input impossible.
    pub(super) peer_eof: CancellationToken,
    /// Actor that schedules and delivers handler responses.
    pub(super) response_actor: ConnectionResponseActor,
    /// Coordinator consulted for each decoder failure.
    pub(super) recovery: RecoveryCoordinator<'a>,
    /// Cancels [`Self::shutdown`] when stream processing exits.
    _cancel_on_exit: CancellationGuard,
}

/// Inputs that decide one connection's lifetime and recovery behaviour.
pub(super) struct RuntimeParams<'a, F: FrameCodec> {
    /// Protocol whose response hooks drive the connection actor.
    ///
    /// The handle is cloned rather than borrowed: the stream context hands its
    /// protocol hooks to the inbound state, so it cannot stay borrowed here.
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Server shutdown signal inherited by this connection.
    pub(super) server_shutdown: &'a CancellationToken,
    /// Hook consulted when a decoder fails on this connection.
    pub(super) recovery_hook: Option<&'a dyn RecoveryPolicyHook>,
    /// Global recovery settings copied for this connection.
    pub(super) recovery_config: RecoveryConfig,
    /// Peer address of the accepted connection, when the transport reported one.
    pub(super) peer_address: Option<SocketAddr>,
}

impl<'a> ConnectionRuntime<'a> {
    /// Build the handles for one accepted connection.
    ///
    /// The shutdown guard is stored in the returned value, so dropping the
    /// runtime cancels the connection's token exactly once.
    pub(super) fn prepare<F: FrameCodec>(params: RuntimeParams<'a, F>) -> io::Result<Self> {
        // A child token so the returned guard cancels only this connection,
        // while a server shutdown still reaches it through the parent.
        let shutdown = params.server_shutdown.child_token();
        // Cancelled only by a clean EOF, which is permanent for the connection.
        let peer_eof = CancellationToken::new();
        let (response_actor, cancel_on_exit) =
            build_connection_actor::<F>(params.protocol.as_ref(), shutdown.clone())?;
        let recovery = RecoveryCoordinator::new(params.recovery_hook, params.recovery_config)
            .with_peer_address(params.peer_address);
        Ok(Self {
            shutdown,
            peer_eof,
            response_actor,
            recovery,
            _cancel_on_exit: cancel_on_exit,
        })
    }
}
