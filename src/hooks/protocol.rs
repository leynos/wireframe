//! The explicit protocol-hook representation used by connections.

use std::sync::Arc;

use super::{
    ConnectionContext,
    CustomProtocolHooks,
    WireframeProtocol,
    dispatch::ProtocolDispatch,
};
use crate::{codec::EofError, push::PushHandle};

/// Callbacks used by the connection actor.
///
/// The three variants make the ownership explicit:
///
/// - [`ProtocolHooks::None`] runs no callbacks;
/// - [`ProtocolHooks::Protocol`] owns one strong reference to an installed [`WireframeProtocol`]
///   and dispatches every operation to it;
/// - [`ProtocolHooks::Custom`] owns independently registered callbacks.
///
/// A protocol-backed connection therefore holds one strong protocol reference,
/// not one per hook. The protocol object itself may be shared by independent
/// connections, so its own fields cannot hold per-connection state. Callbacks
/// on one connection share a single [`ConnectionContext`], which is the
/// designated home for that state; it is currently empty, so callbacks cannot
/// store connection-local state yet.
/// Setup runs at most once per value; the remaining hooks run on every call.
pub enum ProtocolHooks<F, E> {
    /// No hooks are installed.
    None,
    /// One installed protocol owns every operation.
    Protocol(ProtocolDispatch<F, E>),
    /// Independently registered callbacks.
    Custom(CustomProtocolHooks<F, E>),
}

/// The default hooks install no callbacks.
///
/// Written by hand rather than derived so the impl carries no bounds on `F` or
/// `E`. Callers such as [`crate::connection::ConnectionActor::new`] construct
/// the default value under only their own frame and error bounds, which do not
/// imply `Default` for either type parameter.
impl<F, E> Default for ProtocolHooks<F, E> {
    fn default() -> Self { Self::None }
}

impl<F, E> From<CustomProtocolHooks<F, E>> for ProtocolHooks<F, E> {
    /// Wrap independently registered callbacks as connection hooks.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes::Bytes;
    /// use wireframe::hooks::{ConnectionContext, CustomProtocolHooks, ProtocolHooks};
    ///
    /// let custom = CustomProtocolHooks::<Bytes, ()> {
    ///     before_send: Some(Box::new(
    ///         |frame: &mut Bytes, _ctx: &mut ConnectionContext| {
    ///             let mut tagged = frame.to_vec();
    ///             tagged.push(0xff);
    ///             *frame = Bytes::from(tagged);
    ///         },
    ///     )),
    ///     ..CustomProtocolHooks::default()
    /// };
    /// let mut hooks: ProtocolHooks<Bytes, ()> = custom.into();
    ///
    /// let mut frame = Bytes::from_static(b"payload");
    /// hooks.before_send(&mut frame, &mut ConnectionContext::default());
    /// assert_eq!(frame.as_ref(), b"payload\xff".as_slice());
    /// ```
    fn from(hooks: CustomProtocolHooks<F, E>) -> Self { Self::Custom(hooks) }
}

impl<F, E> ProtocolHooks<F, E> {
    /// Run the `on_connection_setup` hook if registered.
    ///
    /// A protocol-backed value forwards setup at most once; a custom value
    /// consumes its setup callback on the first call.
    pub fn on_connection_setup(&mut self, handle: PushHandle<F>, ctx: &mut ConnectionContext) {
        match self {
            Self::None => {}
            Self::Protocol(dispatch) => dispatch.on_connection_setup(handle, ctx),
            Self::Custom(custom) => custom.on_connection_setup(handle, ctx),
        }
    }

    /// Run the `before_send` hook if registered.
    pub fn before_send(&mut self, frame: &mut F, ctx: &mut ConnectionContext) {
        match self {
            Self::None => {}
            Self::Protocol(dispatch) => dispatch.before_send(frame, ctx),
            Self::Custom(custom) => custom.before_send(frame, ctx),
        }
    }

    /// Run the `on_command_end` hook if registered.
    pub fn on_command_end(&mut self, ctx: &mut ConnectionContext) {
        match self {
            Self::None => {}
            Self::Protocol(dispatch) => dispatch.on_command_end(ctx),
            Self::Custom(custom) => custom.on_command_end(ctx),
        }
    }

    /// Run the `handle_error` hook if registered.
    pub fn handle_error(&mut self, error: E, ctx: &mut ConnectionContext) {
        match self {
            Self::None => {}
            Self::Protocol(dispatch) => dispatch.handle_error(error, ctx),
            Self::Custom(custom) => custom.handle_error(error, ctx),
        }
    }

    /// Run the `stream_end_frame` hook if registered.
    pub fn stream_end_frame(&mut self, ctx: &mut ConnectionContext) -> Option<F> {
        match self {
            Self::None => None,
            Self::Protocol(dispatch) => dispatch.stream_end_frame(ctx),
            Self::Custom(custom) => custom.stream_end_frame(ctx),
        }
    }

    /// Run the `on_eof` hook if registered.
    pub fn on_eof(&mut self, error: &EofError, partial_data: &[u8], ctx: &mut ConnectionContext) {
        match self {
            Self::None => {}
            Self::Protocol(dispatch) => dispatch.on_eof(error, partial_data, ctx),
            Self::Custom(custom) => custom.on_eof(error, partial_data, ctx),
        }
    }

    /// Construct hooks that share one installed protocol.
    ///
    /// The returned value owns a single strong reference to `protocol`. Every
    /// connection that installs the same protocol shares that protocol object:
    /// fresh hooks and a fresh [`ConnectionContext`] isolate per-connection
    /// context and callback state, but they do **not** isolate mutable state
    /// stored inside the protocol.
    ///
    /// Setup is forwarded at most once per hooks value. The remaining
    /// operations are forwarded on every call, preserving the trait's
    /// cardinality.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use bytes::Bytes;
    /// use wireframe::hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol};
    ///
    /// struct Tagging;
    ///
    /// impl WireframeProtocol for Tagging {
    ///     type Frame = Bytes;
    ///     type ProtocolError = ();
    ///
    ///     fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {
    ///         let mut tagged = frame.to_vec();
    ///         tagged.push(0xff);
    ///         *frame = Bytes::from(tagged);
    ///     }
    /// }
    ///
    /// let protocol: Arc<Tagging> = Arc::new(Tagging);
    /// let mut hooks = ProtocolHooks::<Bytes, ()>::from_protocol(&protocol);
    ///
    /// let mut frame = Bytes::from_static(b"payload");
    /// hooks.before_send(&mut frame, &mut ConnectionContext::default());
    /// assert_eq!(frame.as_ref(), b"payload\xff".as_slice());
    /// ```
    pub fn from_protocol<P>(protocol: &Arc<P>) -> Self
    where
        P: WireframeProtocol<Frame = F, ProtocolError = E> + ?Sized,
    {
        Self::Protocol(ProtocolDispatch::from_protocol(protocol))
    }
}
