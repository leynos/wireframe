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
/// connections; per-connection mutable state belongs in [`ConnectionContext`].
/// Setup runs at most once per value; the remaining hooks run on every call.
#[derive(Default)]
pub enum ProtocolHooks<F, E> {
    /// No hooks are installed.
    #[default]
    None,
    /// One installed protocol owns every operation.
    Protocol(ProtocolDispatch<F, E>),
    /// Independently registered callbacks.
    Custom(CustomProtocolHooks<F, E>),
}

impl<F, E> From<CustomProtocolHooks<F, E>> for ProtocolHooks<F, E> {
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
    pub fn from_protocol<P>(protocol: &Arc<P>) -> Self
    where
        P: WireframeProtocol<Frame = F, ProtocolError = E> + ?Sized,
    {
        Self::Protocol(ProtocolDispatch::from_protocol(protocol))
    }
}
