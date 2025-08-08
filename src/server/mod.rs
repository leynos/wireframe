//! Tokio-based server for `WireframeApp` instances.
//!
//! `WireframeServer` spawns worker tasks to accept TCP connections,
//! optionally decoding a connection preamble before handing the
//! stream to the application.

use core::marker::PhantomData;
use std::{io, sync::Arc};

use bincode::error::DecodeError;
use futures::future::BoxFuture;
use tokio::{net::TcpListener, sync::oneshot};

use crate::{app::WireframeApp, preamble::Preamble};

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

/// Callback invoked when a connection preamble decodes successfully.
pub type PreambleCallback<T> = Arc<dyn PreambleSuccessHandler<T>>;

/// Callback invoked when decoding a connection preamble fails.
pub type PreambleErrorCallback = Arc<dyn Fn(&DecodeError) + Send + Sync + 'static>;

/// Tokio-based server for [`WireframeApp`] instances.
///
/// The server carries a typestate `S` indicating whether it is
/// [`Unbound`] (not yet bound to a TCP listener) or [`Bound`]. New
/// servers start `Unbound` and must call [`binding::WireframeServer::bind`] or
/// [`binding::WireframeServer::bind_listener`] before running. A worker task is spawned per
/// thread; each receives its own `WireframeApp` from the provided factory
/// closure. The server listens for a shutdown signal using
/// `tokio::signal::ctrl_c` and notifies all workers to stop accepting new
/// connections.
pub struct WireframeServer<F, T = (), S = Unbound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    // `Preamble` covers types implementing `BorrowDecode` for any lifetime,
    // enabling decoding of borrowed data without external context.
    // `()` satisfies this bound via bincode's `BorrowDecode` support for unit,
    // so servers default to having no preamble.
    T: Preamble,
    S: ServerState,
{
    pub(crate) factory: F,
    pub(crate) workers: usize,
    pub(crate) on_preamble_success: Option<PreambleCallback<T>>,
    pub(crate) on_preamble_failure: Option<PreambleErrorCallback>,
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
    /// Typestate tracking whether the server has been bound to a listener.
    /// [`Unbound`] servers require binding before they can run.
    pub(crate) state: S,
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
mod runtime;

/// Re-exported configuration types for server backoff behavior.
pub use runtime::BackoffConfig;

#[cfg(test)]
pub(crate) mod test_util;
