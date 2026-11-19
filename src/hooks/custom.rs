//! Independently registered protocol callbacks.
//!
//! [`CustomProtocolHooks`] is the escape hatch for callers that install
//! separate closures rather than one [`WireframeProtocol`](super::WireframeProtocol).
//! It stores each callback in its own optional slot, converts into
//! [`ProtocolHooks`](super::ProtocolHooks), and runs each callback only when a
//! caller registered it.
//!
//! The type keeps the migration path from the former struct-of-closures
//! representation explicit: existing callers replace a `ProtocolHooks { .. }`
//! literal with `CustomProtocolHooks { .. }.into()`, while a protocol-based
//! application keeps one strong owner instead of six forwarded closures.

use super::ConnectionContext;
use crate::{codec::EofError, push::PushHandle};

/// Type alias for the `before_send` callback.
type BeforeSendHook<F> = Box<dyn FnMut(&mut F, &mut ConnectionContext) + Send + 'static>;

/// Type alias for the `on_connection_setup` callback.
type OnConnectionSetupHook<F> =
    Box<dyn FnOnce(PushHandle<F>, &mut ConnectionContext) + Send + 'static>;

/// Type alias for the `on_command_end` callback.
type OnCommandEndHook = Box<dyn FnMut(&mut ConnectionContext) + Send + 'static>;

/// Type alias for the `handle_error` callback.
type HandleErrorHook<E> = Box<dyn FnMut(E, &mut ConnectionContext) + Send + 'static>;

/// Type alias for the `stream_end_frame` callback.
type StreamEndHook<F> = Box<dyn FnMut(&mut ConnectionContext) -> Option<F> + Send + 'static>;

/// Type alias for the `on_eof` callback.
type OnEofHook = Box<dyn FnMut(&EofError, &[u8], &mut ConnectionContext) + Send + 'static>;

/// Independently registered protocol callbacks.
///
/// Each slot is optional and runs only when a caller registers it. This is the
/// representation for applications that install unrelated closures rather than
/// a single [`super::WireframeProtocol`]. Convert one into
/// [`super::ProtocolHooks`] with [`super::ProtocolHooks::from`] (or `into()`) and pass it to
/// [`crate::connection::ConnectionActor::with_hooks`].
///
/// # Migration
///
/// A value that previously used a structural `ProtocolHooks { .. }` literal is
/// now a [`CustomProtocolHooks`] literal with the same six fields:
///
/// ```
/// use bytes::Bytes;
/// use wireframe::hooks::{ConnectionContext, CustomProtocolHooks, ProtocolHooks};
///
/// let hooks = CustomProtocolHooks {
///     before_send: Some(Box::new(
///         |frame: &mut Bytes, _ctx: &mut ConnectionContext| {
///             let mut tagged = frame.to_vec();
///             tagged.push(0xff);
///             *frame = Bytes::from(tagged);
///         },
///     )),
///     ..CustomProtocolHooks::default()
/// };
/// let mut hooks: ProtocolHooks<Bytes, ()> = hooks.into();
///
/// let mut frame = Bytes::from_static(b"payload");
/// hooks.before_send(&mut frame, &mut ConnectionContext::default());
/// assert_eq!(frame.as_ref(), b"payload\xff".as_slice());
/// ```
pub struct CustomProtocolHooks<F, E> {
    /// Invoked when a connection is established.
    pub on_connection_setup: Option<OnConnectionSetupHook<F>>,
    /// Invoked before a frame is written to the socket.
    pub before_send: Option<BeforeSendHook<F>>,
    /// Invoked once a command completes.
    pub on_command_end: Option<OnCommandEndHook>,
    /// Invoked when a handler returns a protocol error.
    pub handle_error: Option<HandleErrorHook<E>>,
    /// Invoked to construct an end-of-stream frame.
    pub stream_end: Option<StreamEndHook<F>>,
    /// Callback for an EOF condition, run by [`super::ProtocolHooks::on_eof`].
    pub on_eof: Option<OnEofHook>,
}

impl<F, E> Default for CustomProtocolHooks<F, E> {
    fn default() -> Self {
        Self {
            on_connection_setup: None,
            before_send: None,
            on_command_end: None,
            handle_error: None,
            stream_end: None,
            on_eof: None,
        }
    }
}

impl<F, E> CustomProtocolHooks<F, E> {
    /// Run the `on_connection_setup` hook if registered.
    pub(super) fn on_connection_setup(
        &mut self,
        handle: PushHandle<F>,
        ctx: &mut ConnectionContext,
    ) {
        if let Some(hook) = self.on_connection_setup.take() {
            hook(handle, ctx);
        }
    }

    /// Run the `before_send` hook if registered.
    pub(super) fn before_send(&mut self, frame: &mut F, ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.before_send {
            hook(frame, ctx);
        }
    }

    /// Run the `on_command_end` hook if registered.
    pub(super) fn on_command_end(&mut self, ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.on_command_end {
            hook(ctx);
        }
    }

    /// Run the `handle_error` hook if registered.
    pub(super) fn handle_error(&mut self, error: E, ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.handle_error {
            hook(error, ctx);
        }
    }

    /// Run the `stream_end_frame` hook if registered.
    pub(super) fn stream_end_frame(&mut self, ctx: &mut ConnectionContext) -> Option<F> {
        self.stream_end.as_mut().and_then(|hook| hook(ctx))
    }

    /// Run the `on_eof` hook if registered.
    pub(super) fn on_eof(
        &mut self,
        error: &EofError,
        partial_data: &[u8],
        ctx: &mut ConnectionContext,
    ) {
        if let Some(hook) = &mut self.on_eof {
            hook(error, partial_data, ctx);
        }
    }
}
