//! Per-connection output actor used by the inbound frame loop.
//!
//! Application responses are installed on an untracked [`ConnectionActor`]
//! instead of being written directly. The actor owns the biased selection
//! rules that interleave pushed frames with response streams, so a handler
//! that returns `Response::Stream` or `Response::MultiPacket` can be driven to
//! completion while pushes keep flowing.
//!
//! The actor deliberately receives no fragmenter and no `before_send` hook.
//! Fragmentation, codec wrapping, and protocol hooks for application frames
//! all happen in the transport sink owned by the caller, which keeps a single
//! ordering for serialization, hooks, and write metrics.

use std::{io, sync::Arc};

use log::warn;
use tokio_util::sync::CancellationToken;

use super::core::parse_envelope;
use crate::{
    app::Envelope,
    codec::FrameCodec,
    connection::{ConnectionActor, ConnectionChannels},
    frame::FrameMetadata,
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    message::DecodeWith,
    push::{PushHandle, PushQueues},
    serializer::Serializer,
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
pub(super) fn build_connection_actor<S, F>(
    protocol: Option<&Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    serializer: Arc<S>,
    shutdown: CancellationToken,
) -> io::Result<(ConnectionResponseActor, CancellationGuard)>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    F: FrameCodec,
    Envelope: DecodeWith<S>,
{
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(INTERNAL_PUSH_CAPACITY)
        .low_capacity(INTERNAL_PUSH_CAPACITY)
        .build()
        .map_err(|error| io::Error::other(format!("failed to build actor queues: {error}")))?;
    let hooks = protocol.map_or_else(ProtocolHooks::default, |registered| {
        actor_hooks::<S, F>(registered, serializer)
    });
    let actor = ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle.clone()),
        None,
        shutdown.clone(),
        hooks,
    );
    Ok((
        ConnectionResponseActor {
            actor,
            _push_handle: handle,
        },
        CancellationGuard::new(shutdown),
    ))
}

/// Adapt a protocol into the hooks the actor needs for response frames.
///
/// Only the response-facing callbacks are forwarded. `before_send` stays unset
/// because the transport sink runs it after serialization, and setup/EOF are
/// driven by the server rather than by this actor.
fn actor_hooks<S, F>(
    protocol: &Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>,
    serializer: Arc<S>,
) -> ProtocolHooks<Envelope, ()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    F: FrameCodec,
    Envelope: DecodeWith<S>,
{
    let command_end = {
        let command_protocol = Arc::clone(protocol);
        Box::new(move |ctx: &mut ConnectionContext| command_protocol.on_command_end(ctx))
    };
    let handle_error = {
        let error_protocol = Arc::clone(protocol);
        Box::new(move |error: (), ctx: &mut ConnectionContext| {
            error_protocol.handle_error(error, ctx);
        })
    };
    let stream_end = {
        let terminator_protocol = Arc::clone(protocol);
        Box::new(move |ctx: &mut ConnectionContext| {
            let frame = terminator_protocol.stream_end_frame(ctx)?;
            decode_stream_end::<S, F>(serializer.as_ref(), &frame)
        })
    };

    ProtocolHooks {
        on_command_end: Some(command_end),
        handle_error: Some(handle_error),
        stream_end: Some(stream_end),
        ..ProtocolHooks::default()
    }
}

/// Decode a protocol terminator frame into the actor's envelope form.
///
/// A frame that cannot be decoded is logged and omitted rather than closing an
/// otherwise healthy connection.
fn decode_stream_end<S, F>(serializer: &S, frame: &F::Frame) -> Option<Envelope>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    F: FrameCodec,
    Envelope: DecodeWith<S>,
{
    match parse_envelope(serializer, F::frame_payload(frame)) {
        Ok((envelope, _)) => Some(envelope),
        Err(error) => {
            warn!("failed to decode stream-end frame: error={error:?}");
            None
        }
    }
}
