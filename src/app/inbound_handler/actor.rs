//! Per-connection output actor used by the inbound frame loop.
//!
//! Application responses are installed on an untracked [`ConnectionActor`]
//! instead of being written directly. The actor owns the biased selection
//! rules that interleave pushed frames with response streams, so a handler
//! that returns `Response::Stream` or `Response::MultiPacket` is driven to
//! completion while server pushes keep flowing.
//!
//! The actor deliberately receives no fragmenter and no protocol hooks.
//! Fragmentation, codec wrapping, and every protocol callback for application
//! frames — `before_send`, the stream terminator, the error hook and
//! completion — run through the connection's [`FramePipeline`] in the
//! transport sink owned by the caller, which keeps a single ordering for
//! serialization, hooks, and write metrics.
//!
//! Connection setup does run at this boundary, because the connection's push
//! handle and registry entry come into existence here.
//!
//! [`FramePipeline`]: crate::app::codec_driver::FramePipeline

use std::{io, sync::Arc};

use log::debug;
use tokio_util::sync::CancellationToken;

use crate::{
    app::Envelope,
    codec::FrameCodec,
connection::{ConnectionActor, ConnectionChannels, PEER_CLOSED_RESPONSE_GRACE},
    hooks::{ConnectionContext, ProtocolHooks},
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

impl<F: FrameCodec> ConnectionResponseActor<F> {
    /// Identity this connection reports when it abandons a push write.
    pub(super) const fn push_diagnostics(&self) -> crate::app::PushWriteDiagnostics {
        crate::app::PushWriteDiagnostics::new(self.actor.connection_id(), self.actor.peer_addr())
    }
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
    /// Handle published under that identifier.
    handle: PushHandle<F>,
}

impl<F: FrameLike> RegistryGuard<F> {
    /// Wrap the registry entry so it is removed when the guard drops.
    const fn new(
        registry: Arc<SessionRegistry<F>>,
        connection_id: ConnectionId,
        handle: PushHandle<F>,
    ) -> Self {
        Self {
            registry,
            connection_id,
            handle,
        }
    }
}

impl<F: FrameLike> Drop for RegistryGuard<F> {
    fn drop(&mut self) {
        // Only this connection's own entry is removed: a manual insert may have
        // reused the identifier once this connection's entry died, and that
        // handle belongs to someone else. A lookup that finds nothing is
        // indistinguishable from a connection that never registered, so both
        // edges of the entry's lifetime are logged with the identifier that
        // joins them.
        let removed = self.registry.remove_owned(self.connection_id, &self.handle);
        debug!(
            "session registry entry released: connection_id={:?}, removed={removed}",
            self.connection_id
        );
    }
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
    /// Hooks whose setup callback runs once the connection has an identifier.
    ///
    /// Borrowed rather than owned so setup dispatches through the one strong
    /// protocol reference the hooks already hold. The caller moves the same
    /// value into the connection's pipeline after this call returns.
    pub(super) protocol_hooks: &'a mut ProtocolHooks<F::Frame, ()>,
    /// Token cancelled when the connection loop exits.
    pub(super) shutdown: CancellationToken,
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
/// routing. The identifier is allocated and published as one step before the
/// protocol setup callback runs, so setup can address its own connection — or
/// any other live one — through [`SessionRegistry::get`].
///
/// Returns the actor bundle, the guard that cancels the connection token when
/// processing ends, and the connection context the setup callback observed.
///
/// The returned context is the value the caller installs in the frame
/// pipeline, so the state setup leaves in it is the state every later callback
/// sees. The actor holds no copy of it: it receives only the connection
/// identifier and peer address it logs with, and its hooks are inert, so the
/// pipeline's context is the connection's one hook context.
pub(super) fn build_connection_actor<F>(
    init: ConnectionActorInit<'_, F>,
) -> io::Result<(
    ConnectionResponseActor<F>,
    CancellationGuard,
    ConnectionContext,
)>
where
    F: FrameCodec,
{
    let ConnectionActorInit {
        protocol_hooks,
        shutdown,
        registry,
        push_queues,
        context,
    } = init;

    let (queues, handle) = push_queues.clone().build().map_err(|error| {
        io::Error::other(format!("failed to build connection push queues: {error}"))
    })?;
    // Allocating and publishing as one step means the identifier is this
    // connection's own: a manual insert racing it is skipped rather than
    // leaving the connection addressing a handle it does not own. Published
    // before setup so the callback can address this connection, and before the
    // actor starts so a push queued during setup is never dropped.
    let connection_id = registry.allocate_and_publish(&handle);
    debug!("session registry entry published: connection_id={connection_id:?}");
    let registry_guard = RegistryGuard::new(Arc::clone(registry), connection_id, handle.clone());

    let mut connection_context = context.with_published_id(connection_id);
// Setup runs through the hooks value itself, so the connection still owns
    // exactly one strong protocol reference while setup is pending. The
    // prepared constructor builds the actor with inert hooks and an empty
    // context: every later callback runs through the connection's
    // [`FramePipeline`] owner, which holds that same single reference and
    // context. Installing the protocol here as well would give the connection a
    // second strong reference, and dispatching through both would run
    // `before_send` twice on the same frame. The actor receives only the
    // identity it logs with, so no copy of the connection's hook context exists
    // outside the pipeline.
    protocol_hooks.on_connection_setup(handle.clone(), &mut connection_context);
    let mut actor = ConnectionActor::new_prepared_untracked(
        ConnectionChannels::new(queues, handle.clone()),
        shutdown.clone(),
        connection_context.connection_id(),
        connection_context.peer_addr(),
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
    let response_actor = ConnectionResponseActor {
        actor,
        _push_handle: handle,
        _registry: registry_guard,
    };
    Ok((
        response_actor,
        CancellationGuard::new(shutdown),
        connection_context,
    ))
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
