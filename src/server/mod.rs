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

/// Callback invoked when a connection preamble decodes successfully.
///
/// The callback may perform asynchronous I/O on the provided stream before the
/// connection is handed off to [`WireframeApp`].
pub type PreambleCallback<T> = Arc<
    dyn for<'a> Fn(&'a T, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync,
>;

/// Callback invoked when decoding a connection preamble fails.
pub type PreambleErrorCallback = Arc<dyn Fn(&DecodeError) + Send + Sync>;

/// Tokio-based server for `WireframeApp` instances.
///
/// `WireframeServer` spawns a worker task per thread. Each worker
/// receives its own `WireframeApp` from the provided factory
/// closure. The server listens for a shutdown signal using
/// `tokio::signal::ctrl_c` and notifies all workers to stop
/// accepting new connections.
pub struct WireframeServer<F, T = ()>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    // `Preamble` covers types implementing `BorrowDecode` for any lifetime,
    // enabling decoding of borrowed data without external context.
    T: Preamble,
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
    pub(crate) listener: Option<Arc<TcpListener>>,
    pub(crate) _preamble: PhantomData<T>,
}

mod config;
mod connection;
mod runtime;

#[cfg(test)]
pub(crate) mod test_util;
