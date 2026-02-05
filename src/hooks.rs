//! Internal protocol hooks invoked by the connection actor.
//!
//! [`ProtocolHooks`] stores optional callbacks executed during output, while
//! [`WireframeProtocol`] exposes the public interface applications implement to
//! configure those callbacks.

use std::sync::Arc;

use crate::{
    codec::EofError,
    push::{FrameLike, PushHandle},
};

/// Per-connection state passed to protocol callbacks.
///
/// This empty struct is intentionally extensible. Future protocol features may
/// require storing connection-local data without breaking existing APIs.
#[derive(Default)]
pub struct ConnectionContext;

/// Trait encapsulating protocol-specific logic and callbacks.
///
/// `WireframeProtocol` allows a custom `ProtocolError` type, but
/// [`crate::app::WireframeApp::with_protocol`] currently requires
/// `ProtocolError = ()` so the protocol can be stored behind dynamic dispatch
/// with a uniform interface. This constraint may be relaxed in a future
/// release.
pub trait WireframeProtocol: Send + Sync + 'static {
    /// Frame type written to the socket.
    type Frame: FrameLike;
    /// Custom error type for protocol operations.
    ///
    /// When installed via [`crate::app::WireframeApp::with_protocol`], this
    /// must currently be `()`.
    type ProtocolError;

    /// Called once when a new connection is established. The provided
    /// [`PushHandle`] may be stored by the implementation to enable
    /// asynchronous server pushes.
    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {}

    /// Invoked before any frame (push or response) is written to the socket.
    fn before_send(&self, _frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {}

    /// Invoked when a request/response cycle completes.
    fn on_command_end(&self, _ctx: &mut ConnectionContext) {}

    /// Called when a handler returns a [`crate::WireframeError::Protocol`].
    ///
    /// ```rust,ignore
    /// use wireframe::{ConnectionContext, WireframeProtocol};
    ///
    /// struct MyProtocol;
    ///
    /// impl WireframeProtocol for MyProtocol {
    ///     type Frame = Vec<u8>;
    ///     type ProtocolError = ();
    ///
    ///     fn handle_error(&self, error: Self::ProtocolError, _ctx: &mut ConnectionContext) {
    ///         tracing::error!(error = %error, "protocol error");
    ///         // Custom handling here
    ///     }
    /// }
    /// ```
    fn handle_error(&self, _error: Self::ProtocolError, _ctx: &mut ConnectionContext) {}

    /// Produce a frame signalling end-of-stream.
    ///
    /// Implementations should set any protocol-specific flag indicating that
    /// no further frames will follow. Returning `None` omits the terminator
    /// frame, and the stream ends silently. The `before_send` hook runs for
    /// this frame if registered.
    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { None }

    /// Called when an EOF condition is detected during frame decoding.
    ///
    /// This hook is invoked when the connection closes unexpectedly (mid-frame
    /// or mid-header) or cleanly (at frame boundary). Implementations can use
    /// this to log diagnostics, update metrics, or perform cleanup.
    ///
    /// The `partial_data` parameter contains any bytes that were received
    /// before EOF. For clean closes, this will be empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use wireframe::{ConnectionContext, EofError, WireframeProtocol};
    ///
    /// struct MyProtocol;
    ///
    /// impl WireframeProtocol for MyProtocol {
    ///     type Frame = Vec<u8>;
    ///     type ProtocolError = ();
    ///
    ///     fn on_eof(&self, error: &EofError, partial_data: &[u8], _ctx: &mut ConnectionContext) {
    ///         match error {
    ///             EofError::CleanClose => tracing::info!("connection closed cleanly"),
    ///             EofError::MidFrame { .. } => {
    ///                 tracing::warn!(
    ///                     partial_bytes = partial_data.len(),
    ///                     "connection closed mid-frame"
    ///                 );
    ///             }
    ///             EofError::MidHeader { .. } => {
    ///                 tracing::warn!("connection closed mid-header");
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    fn on_eof(&self, _error: &EofError, _partial_data: &[u8], _ctx: &mut ConnectionContext) {}
}

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

/// Callbacks used by the connection actor.
pub struct ProtocolHooks<F, E> {
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
    /// Invoked when an EOF condition is detected.
    pub on_eof: Option<OnEofHook>,
}

impl<F, E> Default for ProtocolHooks<F, E> {
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

impl<F, E> ProtocolHooks<F, E> {
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

    /// Run the `handle_error` hook if registered.
    pub fn handle_error(&mut self, error: E, ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.handle_error {
            hook(error, ctx);
        }
    }

    /// Run the `stream_end_frame` hook if registered.
    pub fn stream_end_frame(&mut self, ctx: &mut ConnectionContext) -> Option<F> {
        self.stream_end.as_mut().and_then(|hook| hook(ctx))
    }

    /// Run the `on_eof` hook if registered.
    pub fn on_eof(&mut self, error: &EofError, partial_data: &[u8], ctx: &mut ConnectionContext) {
        if let Some(hook) = &mut self.on_eof {
            hook(error, partial_data, ctx);
        }
    }

    /// Construct hooks from a [`WireframeProtocol`] implementation.
    pub fn from_protocol<P>(protocol: &Arc<P>) -> Self
    where
        P: WireframeProtocol<Frame = F, ProtocolError = E> + ?Sized,
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

        let protocol_error = Arc::clone(protocol);
        let err = Box::new(move |e: P::ProtocolError, ctx: &mut ConnectionContext| {
            protocol_error.handle_error(e, ctx);
        }) as HandleErrorHook<P::ProtocolError>;

        let protocol_stream_end = Arc::clone(protocol);
        let stream_end =
            Box::new(move |ctx: &mut ConnectionContext| protocol_stream_end.stream_end_frame(ctx))
                as StreamEndHook<F>;

        let protocol_eof = Arc::clone(protocol);
        let on_eof = Box::new(
            move |error: &EofError, partial_data: &[u8], ctx: &mut ConnectionContext| {
                protocol_eof.on_eof(error, partial_data, ctx);
            },
        ) as OnEofHook;

        Self {
            on_connection_setup: Some(setup),
            before_send: Some(before),
            on_command_end: Some(end),
            handle_error: Some(err),
            stream_end: Some(stream_end),
            on_eof: Some(on_eof),
        }
    }
}
