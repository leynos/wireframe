//! Tokio-based server for [`WireframeApp`] instances.
//!
//! [`WireframeServer`] spawns worker tasks to accept TCP connections and can
//! decode an optional preamble before handing the stream to the application.

use core::marker::PhantomData;
use std::{io, sync::Arc, time::Duration};

use bincode::error::DecodeError;
use futures::future::BoxFuture;
use tokio::{net::TcpListener, sync::oneshot};

use crate::{
    app::{Envelope, Packet, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    preamble::Preamble,
    serializer::{BincodeSerializer, Serializer},
};

/// Handler invoked when a connection preamble decodes successfully.
///
/// Implementors may perform asynchronous I/O on the provided stream before the
/// connection is handed off to [`WireframeApp`].
///
/// # Examples
/// ```no_run
/// use std::io;
///
/// use futures::future::BoxFuture;
/// use tokio::net::TcpStream;
/// use wireframe::{app::WireframeApp, server::WireframeServer};
///
/// #[derive(bincode::Decode, bincode::BorrowDecode)]
/// struct MyPreamble;
///
/// let _server = WireframeServer::new(|| WireframeApp::default())
///     .with_preamble::<MyPreamble>()
///     .on_preamble_decode_success(
///         |_preamble: &MyPreamble, stream: &mut TcpStream| -> BoxFuture<'_, io::Result<()>> {
///             Box::pin(async move {
///                 // Perform any initial handshake here.
///                 Ok(())
///             })
///         },
///     );
/// ```
pub trait PreambleSuccessHandler<T>:
    for<'a> Fn(&'a T, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
    + Send
    + Sync
    + 'static
{
}

impl<T, F> PreambleSuccessHandler<T> for F where
    F: for<'a> Fn(&'a T, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync
        + 'static
{
}

/// [`PreambleSuccessHandler`] wrapped in `Arc`.
pub type PreambleHandler<T> = Arc<dyn PreambleSuccessHandler<T>>;

/// Handler invoked when decoding a connection preamble fails.
///
/// Implementors may perform asynchronous I/O on the provided stream to emit a
/// response before the connection is closed.
pub type PreambleFailure = Arc<
    dyn for<'a> Fn(&'a DecodeError, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync
        + 'static,
>;

/// Convert a factory output into a `Result` for a `WireframeApp`.
pub trait FactoryResult<App> {
    /// Error type returned when conversion fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Convert the value into a `Result`.
    ///
    /// # Errors
    ///
    /// Returns any error produced while converting into the result form.
    fn into_result(self) -> Result<App, Self::Error>;
}

impl<Ser, Ctx, E, Codec> FactoryResult<WireframeApp<Ser, Ctx, E, Codec>>
    for WireframeApp<Ser, Ctx, E, Codec>
where
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    type Error = std::convert::Infallible;

    fn into_result(self) -> Result<WireframeApp<Ser, Ctx, E, Codec>, Self::Error> { Ok(self) }
}

impl<Ser, Ctx, E, Codec, Err> FactoryResult<WireframeApp<Ser, Ctx, E, Codec>>
    for Result<WireframeApp<Ser, Ctx, E, Codec>, Err>
where
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Err: std::error::Error + Send + Sync + 'static,
{
    type Error = Err;

    fn into_result(self) -> Result<WireframeApp<Ser, Ctx, E, Codec>, Self::Error> { self }
}

/// Factory trait for building `WireframeApp` instances used by the server.
pub trait AppFactory<Ser, Ctx, E, Codec>: Send + Sync + Clone + 'static
where
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    /// Error type returned when the factory cannot construct an application.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Build an application instance for a new connection.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the factory while constructing the app.
    fn build(&self) -> Result<WireframeApp<Ser, Ctx, E, Codec>, Self::Error>;
}

impl<F, R, Ser, Ctx, E, Codec> AppFactory<Ser, Ctx, E, Codec> for F
where
    F: Fn() -> R + Send + Sync + Clone + 'static,
    R: FactoryResult<WireframeApp<Ser, Ctx, E, Codec>>,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    type Error = R::Error;

    fn build(&self) -> Result<WireframeApp<Ser, Ctx, E, Codec>, Self::Error> {
        (self)().into_result()
    }
}

/// Tokio-based server for [`WireframeApp`] instances.
///
/// The server carries a typestate `S` indicating whether it is
/// [`Unbound`] (not yet bound to a TCP listener) or [`Bound`]. New
/// servers start `Unbound` and must call [`WireframeServer::bind`] or
/// [`WireframeServer::bind_existing_listener`] before running. A worker task is spawned
/// per thread; each receives its own `WireframeApp` from the provided factory
/// closure. The server listens for a shutdown signal using
/// `tokio::signal::ctrl_c` and notifies all workers to stop accepting new
/// connections.
///
/// # Examples
/// ```no_run
/// use wireframe::{
///     app::WireframeApp,
///     server::{ServerError, WireframeServer},
/// };
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), ServerError> {
/// // Start unbound (S = Unbound)
/// let srv = WireframeServer::new(|| WireframeApp::default());
///
/// // Transition to bound (S = Bound)
/// let srv = srv.bind("127.0.0.1:0")?;
///
/// // Run the server
/// srv.run().await
/// # }
/// ```
pub struct WireframeServer<
    F,
    T = (),
    S = Unbound,
    Ser = BincodeSerializer,
    Ctx = (),
    E = Envelope,
    Codec = LengthDelimitedFrameCodec,
> where
    F: AppFactory<Ser, Ctx, E, Codec>,
    // `Preamble` covers types implementing `BorrowDecode` for any lifetime,
    // enabling decoding of borrowed data without external context.
    // `()` satisfies this bound via bincode's `BorrowDecode` support for unit,
    // so servers default to having no preamble.
    T: Preamble,
    S: ServerState,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    pub(crate) factory: F,
    pub(crate) workers: usize,
    pub(crate) on_preamble_success: Option<PreambleHandler<T>>,
    pub(crate) on_preamble_failure: Option<PreambleFailure>,
    /// Channel used to notify when the server is ready.
    ///
    /// # Thread Safety
    /// This sender is `Send` and may be moved between threads safely.
    ///
    /// # Single-use Semantics
    /// A `oneshot::Sender` can transmit only one readiness notification. After
    /// sending, the sender is consumed and cannot be reused.
    ///
    /// # Implications
    /// Because only one notification may be sent, a new `ready_tx` must be
    /// provided each time the server is started.
    pub(crate) ready_tx: Option<oneshot::Sender<()>>,
    pub(crate) backoff_config: BackoffConfig,
    /// Maximum duration allowed for reading a preamble before timing out.
    ///
    /// `None` disables the timeout. A zero or sub-millisecond timeout is
    /// normalised to 1 ms when configured.
    pub(crate) preamble_timeout: Option<Duration>,
    /// Typestate tracking whether the server has been bound to a listener.
    /// [`Unbound`] servers require binding before they can run.
    pub(crate) state: S,
    pub(crate) _app: PhantomData<(Ser, Ctx, E, Codec)>,
    pub(crate) _preamble: PhantomData<T>,
}

/// Marker indicating the server has not yet bound a listener.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unbound;

/// Marker indicating the server is bound to a TCP listener.
#[derive(Debug, Clone)]
pub struct Bound {
    pub(crate) listener: Arc<TcpListener>,
}

/// Trait implemented by [`Unbound`] and [`Bound`] to model binding typestate.
pub trait ServerState: sealed::Sealed {}

mod sealed {
    //! Prevent external implementations of [`ServerState`].

    pub trait Sealed {}
    impl Sealed for super::Unbound {}
    impl Sealed for super::Bound {}
}

impl ServerState for Unbound {}
impl ServerState for Bound {}

mod config;
pub use config::{binding, preamble};
mod connection;
pub mod error;
pub use error::ServerError;
mod runtime;

/// Re-exported configuration types for server backoff behavior.
pub use runtime::BackoffConfig;

#[cfg(test)]
pub(crate) mod test_util;
