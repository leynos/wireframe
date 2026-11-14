//! Per-connection output actor used by the inbound frame loop.
//!
//! Application responses are installed on an untracked [`ConnectionActor`]
//! instead of being written directly. The actor owns the biased selection
//! rules that interleave pushed frames with response streams, so a handler
//! that returns `Response::Stream` or `Response::MultiPacket` is driven to
//! completion while server pushes keep flowing.
//!
//! The actor deliberately receives no fragmenter and no `before_send` hook.
//! Fragmentation, codec wrapping, and protocol hooks for application frames
//! all happen in the transport sink owned by the caller, which keeps a single
//! ordering for serialization, hooks, and write metrics.

use std::{io, sync::Arc};

use tokio_util::sync::CancellationToken;

use crate::{
    app::Envelope,
    codec::FrameCodec,
    connection::{
        ConnectionActor,
        ConnectionChannels,
        PEER_CLOSED_RESPONSE_GRACE,
        UntrackedActorInit,
    },
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    push::{FrameLike, PushHandle, PushQueuesBuilder},
    session::{ConnectionId, SessionRegistry},
};

/// Per-connection actor plus the state that keeps its queues and registry
/// entry alive.
///
/// The push handle is retained for the whole connection: the queues are only
/// open while a strong handle exists, and the registry stores a weak reference
/// so lookups succeed exactly while this connection is live.
pub(super) struct ConnectionResponseActor<F: FrameCodec> {
    /// Actor driven by the inbound frame loop.
    pub(super) actor: ConnectionActor<Envelope, (), F::Frame>,
    /// Strong handle keeping the high- and low-priority queues open.
    _push_handle: PushHandle<F::Frame>,
    /// Removes the registry entry when the connection ends.
    _registry: RegistryGuard<F::Frame>,
}

/// Removes a connection's registry entry on drop.
///
/// Teardown is a drop guard rather than an explicit call so every exit path
/// (clean EOF, I/O error, panic unwinding, and task cancellation) deregisters
/// the connection.
struct RegistryGuard<F: FrameLike> {
    /// Registry the connection registered with.
    registry: Arc<SessionRegistry<F>>,
    /// Identifier allocated for this connection.
    connection_id: ConnectionId,
}

impl<F: FrameLike> RegistryGuard<F> {
    /// Wrap the registry entry so it is removed when the guard drops.
    const fn new(registry: Arc<SessionRegistry<F>>, connection_id: ConnectionId) -> Self {
        Self {
            registry,
            connection_id,
        }
    }
}

impl<F: FrameLike> Drop for RegistryGuard<F> {
    fn drop(&mut self) { self.registry.remove(&self.connection_id); }
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

/// Inputs for building a server connection's output actor.
pub(super) struct ConnectionActorInit<'a, F>
where
    F: FrameCodec,
{
    /// Protocol whose setup and response hooks drive the actor.
    pub(super) protocol:
        Option<&'a Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Token cancelled when the connection loop exits.
    pub(super) shutdown: CancellationToken,
    /// Identifier allocated for this connection.
    pub(super) connection_id: ConnectionId,
    /// Registry the connection publishes its push handle to.
    pub(super) registry: &'a Arc<SessionRegistry<F::Frame>>,
    /// Template cloned to build this connection's push queues.
    pub(super) push_queues: &'a PushQueuesBuilder<F::Frame>,
    /// Connection-local state handed to protocol callbacks.
    pub(super) context: ConnectionContext,
}

/// Build the queues, registry entry, and actor for one connection.
///
/// The push queues are built from the application's template, so every
/// connection gets independent capacity, rate-limiter state, and dead-letter
/// routing. The registry entry is published before the protocol setup callback
/// runs, so setup can address its own connection — or any other live one —
/// through [`SessionRegistry::get`].
///
/// Returns the actor bundle and the guard that cancels the connection token
/// when processing ends.
pub(super) fn build_connection_actor<F>(
    init: ConnectionActorInit<'_, F>,
) -> io::Result<(ConnectionResponseActor<F>, CancellationGuard)>
where
    F: FrameCodec,
{
    let ConnectionActorInit {
        protocol,
        shutdown,
        connection_id,
        registry,
        push_queues,
        context,
    } = init;
    // Published before setup so the callback can address this connection, and
    // before the actor starts so a push queued during setup is never dropped.
    registry.insert(connection_id, &handle);
    let registry_guard = RegistryGuard::new(Arc::clone(registry), connection_id);

    let mut connection_context = context;
    if let Some(setup_protocol) = protocol {
        setup_protocol.on_connection_setup(handle.clone(), &mut connection_context);
    }
    let mut response_actor = ConnectionResponseActor {
        actor,
        _push_handle: handle,
        _registry: registry_guard,
    };
    let (queues, handle) = push_queues.clone().build().map_err(|error| {
        io::Error::other(format!("failed to build connection push queues: {error}"))
    })?;

    let hooks = protocol.map_or_else(ProtocolHooks::default, actor_hooks::<F>);
    let actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown: shutdown.clone(),
        hooks,
        ctx: connection_context,
    });
    // The connection boundary owns the idle-grace policy; the actor only needs
    // a bound, not an opinion about how long a vanished peer may hold a
    // response open.
    response_actor
        .actor
        .set_peer_closed_grace(PEER_CLOSED_RESPONSE_GRACE);
    // ADR-014 keeps a connection draining a readable peer across `stop()`, so
    // shutdown must not abandon the response drive here. The stop token still
    // bounds every write, which is what prevents a peer that stopped reading
    // from pinning the drain.
    actor.set_abandon_response_on_shutdown(false);
    Ok((response_actor, CancellationGuard::new(shutdown)))
}

/// Install the per-dispatch peer signal and return the token and cause cell.
///
/// A fresh child token per dispatch inherits a permanent EOF, but cancelling it
/// leaves the connection-wide token untouched, so one buffered request cannot
/// make every later response run under the idle grace. The cause cell is filled
/// when the loop stops polling the read half, so the actor's cancellation
/// warning can name the cause.
pub(super) fn observe_peer<P>(
    actor: &mut ConnectionActor<Envelope, (), P>,
    peer_eof: &CancellationToken,
) -> (
    CancellationToken,
    Arc<std::sync::OnceLock<crate::connection::UnobservedCause>>,
)
where
    P: FrameLike,
{
    let unobserved = peer_eof.child_token();
    let cause = Arc::new(std::sync::OnceLock::new());
    actor.set_peer_observation(unobserved.clone(), Arc::clone(&cause));
    (unobserved, cause)
}

/// Adapt a protocol into the hooks the actor needs for response frames.
///
/// Only the error callback is forwarded. Terminator emission and completion
/// run in the response writer after the actor has finished driving the
/// response: the protocol returns a complete codec frame, and writing that
/// frame unchanged is the only way to preserve metadata a custom codec adds
/// outside the payload. `before_send` stays unset because the transport sink
/// runs it after serialization. The protocol's setup and EOF callbacks are not
/// invoked on the prepared path through this actor, so they are deliberately
/// absent here; setup runs in [`build_connection_actor`].
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
