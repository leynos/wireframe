//! Internal protocol hooks invoked by the connection actor.
//!
//! [`ProtocolHooks`] is an explicit representation of where a connection's
//! callbacks come from. It either installs one [`WireframeProtocol`] as a
//! single shared owner, carries a set of independently registered callbacks
//! ([`CustomProtocolHooks`]), or runs no callbacks at all.
//!
//! A protocol-backed value owns exactly one strong reference to the installed
//! protocol, rather than one clone per hook. The protocol object may be shared
//! by independent connections, so a protocol implementation must not treat its
//! own fields as per-connection state. The crate-private `ConnectionProtocol`
//! pairs the hooks with the single [`ConnectionContext`] a connection reuses
//! for its whole lifetime; that context is the designated home for such state.

use std::net::SocketAddr;

mod connection;

mod custom;

mod dispatch;

mod protocol;

pub(crate) use connection::ConnectionProtocol;
pub use custom::CustomProtocolHooks;
pub use dispatch::ProtocolDispatch;
pub use protocol::ProtocolHooks;

use crate::{
    codec::EofError,
    push::{FrameLike, PushHandle},
    session::ConnectionId,
};

/// Per-connection state passed to protocol callbacks.
///
/// The context is intentionally extensible. Future protocol features may
/// require storing connection-local data without breaking existing APIs, so the
/// struct is non-exhaustive and obtains the connection identifier from the
/// server rather than from a public constructor.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ConnectionContext {
    /// Identifier assigned by the server, absent for standalone actors.
    connection_id: Option<ConnectionId>,
    /// Peer address of the accepted socket, absent when it is not reported.
    peer_addr: Option<SocketAddr>,
}

impl ConnectionContext {
    /// Attach the identifier the connection claimed when it published.
    ///
    /// The identifier is decided at publication, so a context built before the
    /// connection registers cannot carry one.
    pub(crate) const fn with_published_id(mut self, connection_id: ConnectionId) -> Self {
        self.connection_id = Some(connection_id);
        self
    }

    /// Attach the accepted socket's peer address for diagnostics.
    ///
    /// The address is used only to label connection-scoped events; a transport
    /// that cannot report one, such as an in-memory duplex stream, leaves it
    /// absent.
    pub(crate) const fn with_peer_addr(mut self, peer_addr: Option<SocketAddr>) -> Self {
        self.peer_addr = peer_addr;
        self
    }

    /// Return the identifier of the connection this context belongs to.
    ///
    /// The identifier is present for connections accepted by
    /// [`crate::server::WireframeServer`] and absent for a
    /// [`crate::connection::ConnectionActor`] driven directly by an
    /// application or test.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::hooks::ConnectionContext;
    ///
    /// assert_eq!(ConnectionContext::default().connection_id(), None);
    /// ```
    #[must_use]
    pub const fn connection_id(&self) -> Option<ConnectionId> { self.connection_id }

    /// Return the peer address of the accepted socket, when it is known.
    ///
    /// The address is present for connections accepted by
    /// [`crate::server::WireframeServer`] and absent for standalone actors or
    /// transports without a peer, so it can label connection diagnostics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::hooks::ConnectionContext;
    ///
    /// assert_eq!(ConnectionContext::default().peer_addr(), None);
    /// ```
    #[must_use]
    pub const fn peer_addr(&self) -> Option<SocketAddr> { self.peer_addr }
}

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
    ///
    /// On a prepared server connection this runs once per accepted connection,
    /// after the handle is published to the application's
    /// [`SessionRegistry`][crate::session::SessionRegistry] and before the
    /// actor starts, so frames queued here are delivered. The context carries
    /// the server-allocated [`ConnectionId`].
    /// Code that drives a [`ConnectionActor`][crate::connection::ConnectionActor]
    /// directly invokes the callback itself.
    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {}

    /// Invoked before application-response and connection-actor frames are
    /// written to the socket.
    ///
    /// The frame representation depends on which producer emits it:
    ///
    /// - For [`crate::app::WireframeApp`] handler responses, the hook receives the serialized,
    ///   codec-wrapped frame before codec encoding: the payload is fragmented if configured, then
    ///   serialized and passed through [`FrameCodec::wrap_payload`] before the hook runs. A custom
    ///   encoder may still add its own header or trailer after the hook runs, so the hook sees the
    ///   frame payload rather than the exact bytes written to the socket.
    /// - For frames emitted by the connection actor, such as server pushes, multi-packet channel
    ///   frames, and `stream_end_frame` output, the hook receives the frame in its already-built
    ///   `Self::Frame` form. `ConnectionActor::push_frame` runs the hook on the frame and then
    ///   appends it to the output; it does not serialize or codec-wrap the frame itself.
    ///
    /// The direct response helpers on [`crate::app::WireframeApp`]
    /// (`send_response`, `send_response_framed`, and
    /// `send_response_framed_with_codec`) write to a caller-supplied stream or
    /// framed sink instead of the connection output, and do not invoke this
    /// hook.
    ///
    /// In both of the paths above the hook may enlarge the frame (for example,
    /// to stamp a trailer). There is no implicit allowance for that growth:
    /// implementations **must** leave the frame within the codec's
    /// [`FrameCodec::max_frame_length`]. The default
    /// [`LengthDelimitedFrameCodec`] rejects an oversized frame with
    /// [`FramingError::OversizedFrame`], which surfaces as a send failure and
    /// ends the connection. A custom codec defines its own behaviour: it may
    /// return a different error, or not enforce the limit at all. Fragmented
    /// output is sized below the frame budget, but the resulting slack belongs
    /// to fragmentation overhead rather than to hooks.
    ///
    /// [`FrameCodec::wrap_payload`]: crate::codec::FrameCodec::wrap_payload
    /// [`FrameCodec::max_frame_length`]: crate::codec::FrameCodec::max_frame_length
    /// [`LengthDelimitedFrameCodec`]: crate::codec::LengthDelimitedFrameCodec
    /// [`FramingError::OversizedFrame`]: crate::codec::FramingError::OversizedFrame
    fn before_send(&self, _frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {}

    /// Invoked when a request/response cycle completes.
    fn on_command_end(&self, _ctx: &mut ConnectionContext) {}

    /// Called when a handler returns a [`crate::WireframeError::Protocol`].
    ///
    /// ```rust,ignore
    /// use wireframe::hooks::{ConnectionContext, WireframeProtocol};
    ///
    /// struct MyProtocol;
    ///
    /// impl WireframeProtocol for MyProtocol {
    ///     type Frame = Vec<u8>;
    ///     type ProtocolError = ();
    ///
    ///     fn handle_error(&self, _error: Self::ProtocolError, _ctx: &mut ConnectionContext) {
    ///         tracing::error!("protocol error");
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

    /// Called when a connection's input ends part-way through a frame.
    ///
    /// A prepared server connection calls this hook once when its decoder
    /// reports that the peer closed mid-header or mid-frame, passing the
    /// structured [`EofError`] and the bytes that had arrived for the frame
    /// that never completed. A close at a frame boundary is a clean close and
    /// does not invoke the hook, so a call is evidence that data was lost.
    /// Transport failures and decode errors end the connection through their
    /// own paths and do not reach this hook either.
    ///
    /// Code that drives a [`ConnectionActor`][crate::connection::ConnectionActor]
    /// directly invokes the callback itself, either on this trait method or
    /// through [`ProtocolHooks::on_eof`].
    ///
    /// The `partial_data` parameter contains every byte the framed reader
    /// still held for the abandoned frame, including its length prefix. A
    /// custom decoder that consumes bytes before reporting its end-of-file
    /// condition yields only the bytes it left behind.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use wireframe::{
    ///     codec::EofError,
    ///     hooks::{ConnectionContext, WireframeProtocol},
    /// };
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
