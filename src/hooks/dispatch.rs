//! Type erasure and dispatch for one installed protocol.

use std::sync::Arc;

use super::{ConnectionContext, WireframeProtocol};
use crate::{codec::EofError, push::PushHandle};

/// Object-safe view of the six protocol operations.
///
/// The public [`super::ProtocolDispatch`] erases the concrete protocol type
/// through this trait so that a [`super::ProtocolHooks`] value can own one
/// protocol behind a single allocation while still dispatching every
/// operation.
trait ProtocolOperations<F, E>: Send {
    /// Run the connection setup operation.
    fn on_connection_setup(&mut self, handle: PushHandle<F>, ctx: &mut ConnectionContext);
    /// Run the outbound frame operation.
    fn before_send(&mut self, frame: &mut F, ctx: &mut ConnectionContext);
    /// Run the command-completion operation.
    fn on_command_end(&mut self, ctx: &mut ConnectionContext);
    /// Run the protocol-error operation.
    fn handle_error(&mut self, error: E, ctx: &mut ConnectionContext);
    /// Produce an end-of-stream frame.
    fn stream_end_frame(&mut self, ctx: &mut ConnectionContext) -> Option<F>;
    /// Run the explicit EOF operation.
    fn on_eof(&mut self, error: &EofError, partial_data: &[u8], ctx: &mut ConnectionContext);
}

/// Sized adapter that owns the single strong reference to an installed
/// protocol.
///
/// The adapter consumes setup on the first call and forwards every other
/// operation on every call, matching the cardinality of the trait's default
/// methods and of the closure-backed [`super::CustomProtocolHooks`].
struct ProtocolAdapter<P: ?Sized> {
    /// The single strong reference held on behalf of the connection.
    protocol: Arc<P>,
    /// Set once setup has been forwarded so later calls are ignored.
    setup_consumed: bool,
}

impl<P, F, E> ProtocolOperations<F, E> for ProtocolAdapter<P>
where
    P: WireframeProtocol<Frame = F, ProtocolError = E> + ?Sized,
{
    fn on_connection_setup(&mut self, handle: PushHandle<F>, ctx: &mut ConnectionContext) {
        if self.setup_consumed {
            return;
        }
        self.setup_consumed = true;
        self.protocol.on_connection_setup(handle, ctx);
    }

    fn before_send(&mut self, frame: &mut F, ctx: &mut ConnectionContext) {
        self.protocol.before_send(frame, ctx);
    }

    fn on_command_end(&mut self, ctx: &mut ConnectionContext) { self.protocol.on_command_end(ctx); }

    fn handle_error(&mut self, error: E, ctx: &mut ConnectionContext) {
        self.protocol.handle_error(error, ctx);
    }

    fn stream_end_frame(&mut self, ctx: &mut ConnectionContext) -> Option<F> {
        self.protocol.stream_end_frame(ctx)
    }

    fn on_eof(&mut self, error: &EofError, partial_data: &[u8], ctx: &mut ConnectionContext) {
        self.protocol.on_eof(error, partial_data, ctx);
    }
}

/// Type-erased dispatcher for one installed protocol.
///
/// A value owns exactly one strong reference to the installed protocol. It has
/// no public constructor: [`super::ProtocolHooks::from_protocol`] is the only
/// way to build one, which keeps the erased adapter an implementation detail.
pub struct ProtocolDispatch<F, E> {
    /// The erased, singly-owned protocol operations.
    operations: Box<dyn ProtocolOperations<F, E> + Send>,
}

impl<F, E> ProtocolDispatch<F, E> {
    /// Erase the concrete protocol type behind a single owned `Arc` clone.
    pub(crate) fn from_protocol<P>(protocol: &Arc<P>) -> Self
    where
        P: WireframeProtocol<Frame = F, ProtocolError = E> + ?Sized,
    {
        Self {
            operations: Box::new(ProtocolAdapter {
                protocol: Arc::clone(protocol),
                setup_consumed: false,
            }),
        }
    }

    /// Run the `on_connection_setup` operation at most once.
    pub(crate) fn on_connection_setup(
        &mut self,
        handle: PushHandle<F>,
        ctx: &mut ConnectionContext,
    ) {
        self.operations.on_connection_setup(handle, ctx);
    }

    /// Run the `before_send` operation.
    pub(crate) fn before_send(&mut self, frame: &mut F, ctx: &mut ConnectionContext) {
        self.operations.before_send(frame, ctx);
    }

    /// Run the `on_command_end` operation.
    pub(crate) fn on_command_end(&mut self, ctx: &mut ConnectionContext) {
        self.operations.on_command_end(ctx);
    }

    /// Run the `handle_error` operation.
    pub(crate) fn handle_error(&mut self, error: E, ctx: &mut ConnectionContext) {
        self.operations.handle_error(error, ctx);
    }

    /// Run the `stream_end_frame` operation.
    pub(crate) fn stream_end_frame(&mut self, ctx: &mut ConnectionContext) -> Option<F> {
        self.operations.stream_end_frame(ctx)
    }

    /// Run the `on_eof` operation.
    pub(crate) fn on_eof(
        &mut self,
        error: &EofError,
        partial_data: &[u8],
        ctx: &mut ConnectionContext,
    ) {
        self.operations.on_eof(error, partial_data, ctx);
    }
}
