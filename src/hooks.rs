//! Internal protocol hooks called by the connection actor.
//!
//! This module defines [`ProtocolHooks`] along with the public
//! [`WireframeProtocol`] trait. `ProtocolHooks` stores optional callbacks
//! invoked during connection output. Applications configure these callbacks via
//! an implementation of [`WireframeProtocol`].

use std::sync::Arc;

use crate::push::{FrameLike, PushHandle};

/// Per-connection state passed to protocol callbacks.
///
/// This empty struct is intentionally extensible. Future protocol features may
/// require storing connection-local data without breaking existing APIs.
#[derive(Default)]
pub struct ConnectionContext;

/// Trait encapsulating protocol-specific logic and callbacks.
pub trait WireframeProtocol: Send + Sync + 'static {
    /// Frame type written to the socket.
    type Frame: FrameLike;
    /// Custom error type for protocol operations.
    type ProtocolError;

    /// Called once when a new connection is established. The provided
    /// [`PushHandle`] may be stored by the implementation to enable
    /// asynchronous server pushes.
    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {}

    /// Invoked before any frame (push or response) is written to the socket.
    fn before_send(&self, _frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {}

    /// Invoked when a request/response cycle completes.
    fn on_command_end(&self, _ctx: &mut ConnectionContext) {}
}

/// Type alias for the `before_send` callback.
type BeforeSendHook<F> = Box<dyn FnMut(&mut F, &mut ConnectionContext) + Send + 'static>;

/// Type alias for the `on_connection_setup` callback.
type OnConnectionSetupHook<F> =
    Box<dyn FnOnce(PushHandle<F>, &mut ConnectionContext) + Send + 'static>;

/// Type alias for the `on_command_end` callback.
type OnCommandEndHook = Box<dyn FnMut(&mut ConnectionContext) + Send + 'static>;

/// Callbacks used by the connection actor.
pub struct ProtocolHooks<F> {
    /// Invoked when a connection is established.
    pub on_connection_setup: Option<OnConnectionSetupHook<F>>,
    /// Invoked before a frame is written to the socket.
    pub before_send: Option<BeforeSendHook<F>>,
    /// Invoked once a command completes.
    pub on_command_end: Option<OnCommandEndHook>,
}

impl<F> Default for ProtocolHooks<F> {
    fn default() -> Self {
        Self {
            on_connection_setup: None,
            before_send: None,
            on_command_end: None,
        }
    }
}

impl<F> ProtocolHooks<F> {
    /// Run the `on_connection_setup` hook if registered.
    pub fn on_connection_setup(&mut self, handle: PushHandle<F>, ctx: &mut ConnectionContext) {
        if let Some(hook) = self.on_connection_setup.take() {
            hook(handle, ctx);
        }
    }
    /// Run the `before_send` hook if registered.
    pub fn before_send(&mut self, frame: &mut F, ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.before_send {
            hook(frame, ctx);
        }
    }

    /// Run the `on_command_end` hook if registered.
    pub fn on_command_end(&mut self, ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.on_command_end {
            hook(ctx);
        }
    }

    /// Construct hooks from a [`WireframeProtocol`] implementation.
    pub fn from_protocol<P>(protocol: &Arc<P>) -> Self
    where
        P: WireframeProtocol<Frame = F> + ?Sized,
    {
        let protocol_before = Arc::clone(protocol);
        let before = Box::new(move |frame: &mut F, ctx: &mut ConnectionContext| {
            protocol_before.before_send(frame, ctx);
        }) as BeforeSendHook<F>;

        let protocol_end = Arc::clone(protocol);
        let end = Box::new(move |ctx: &mut ConnectionContext| {
            protocol_end.on_command_end(ctx);
        }) as OnCommandEndHook;

        let protocol_setup = Arc::clone(protocol);
        let setup = Box::new(move |handle: PushHandle<F>, ctx: &mut ConnectionContext| {
            protocol_setup.on_connection_setup(handle, ctx);
        }) as OnConnectionSetupHook<F>;

        Self {
            on_connection_setup: Some(setup),
            before_send: Some(before),
            on_command_end: Some(end),
        }
    }
}
