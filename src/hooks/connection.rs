//! The per-connection owner of protocol hooks and their context.
//!
//! This module provides [`ConnectionProtocol`], the pair a connection keeps for
//! its whole life: the hooks described by [`ProtocolHooks`] and the single
//! [`ConnectionContext`] those callbacks observe. It exists so a connection
//! cannot accidentally hold hooks and context separately and pass mismatched
//! halves to different callbacks.
//!
//! The utility is a stable dispatch identity. Whichever path owns the value —
//! the frame pipeline on the prepared server path, or the actor when driven
//! directly — every callback on one connection sees the same context in the
//! documented order, without an `Arc<Mutex<_>>` around it.

use super::{ConnectionContext, ProtocolHooks};
use crate::{codec::EofError, push::PushHandle};

/// One connection's protocol hooks and the context they share.
///
/// Exactly one value provides the callbacks for a connection: the frame
/// pipeline on the prepared server path, or the connection actor when an actor
/// is driven directly through its own hooks. Every protocol operation on that
/// path therefore observes the same [`ConnectionContext`] in the documented
/// order. The value is deliberately not `Clone` and does not wrap the context
/// in `Arc<Mutex<_>>`: the owning path invokes hooks one at a time.
///
/// A prepared connection's actor holds no `ConnectionProtocol` at all: it is
/// built in a no-hook mode, so it keeps no protocol reference and no copy of
/// the connection's context. The connection's hook context lives only in the
/// pipeline, which is the only hook owner that dispatches for that connection.
pub(crate) struct ConnectionProtocol<F, E> {
    /// Hooks invoked for this connection.
    hooks: ProtocolHooks<F, E>,
    /// The single context reused for the connection's lifetime.
    context: ConnectionContext,
}

impl<F, E> ConnectionProtocol<F, E> {
    /// Pair hooks with a fresh default context.
    pub(crate) fn new(hooks: ProtocolHooks<F, E>) -> Self {
        Self {
            hooks,
            context: ConnectionContext::default(),
        }
    }

    /// Pair hooks with a context the connection runtime already prepared.
    ///
    /// The server allocates the connection identifier before the actor exists,
    /// so the context arrives populated. Taking it here, rather than building a
    /// fresh one, is what keeps every callback on the connection observing the
    /// same identity.
    pub(crate) const fn with_context(
        hooks: ProtocolHooks<F, E>,
        context: ConnectionContext,
    ) -> Self {
        Self { hooks, context }
    }

    /// Replace the context the callbacks observe.
    pub(crate) const fn set_context(&mut self, context: ConnectionContext) {
        self.context = context;
    }

    /// Run connection setup against this connection's context.
    pub(crate) fn on_connection_setup(&mut self, handle: PushHandle<F>) {
        self.hooks.on_connection_setup(handle, &mut self.context);
    }

    /// Run the outbound frame hook against this connection's context.
    pub(crate) fn before_send(&mut self, frame: &mut F) {
        self.hooks.before_send(frame, &mut self.context);
    }

    /// Run the end-of-file hook against this connection's context.
    pub(crate) fn on_eof(&mut self, error: &EofError, partial_data: &[u8]) {
        self.hooks.on_eof(error, partial_data, &mut self.context);
    }

    /// Run the command-completion hook against this connection's context.
    pub(crate) fn on_command_end(&mut self) { self.hooks.on_command_end(&mut self.context); }

    /// Run the protocol-error hook against this connection's context.
    pub(crate) fn handle_error(&mut self, error: E) {
        self.hooks.handle_error(error, &mut self.context);
    }

    /// Ask the protocol for a stream-end frame against this connection's
    /// context.
    pub(crate) fn stream_end_frame(&mut self) -> Option<F> {
        self.hooks.stream_end_frame(&mut self.context)
    }
}
