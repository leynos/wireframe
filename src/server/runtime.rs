//! Runtime control for [`WireframeServer`].

mod backoff;
#[cfg(test)]
mod tests;

use std::{fmt, io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
pub use backoff::BackoffConfig;
use futures::Future;
use log::warn;
use tokio::{
    net::{TcpListener, TcpStream},
    select,
    signal,
    time::{Duration, sleep},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    Bound,
    PreambleFailure,
    PreambleHandler,
    ServerError,
    WireframeServer,
    connection::{ConnectionApp, spawn_connection_task},
};
use crate::{
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    frame::FrameMetadata,
    preamble::Preamble,
    serializer::Serializer,
};

/// Abstraction for sources of incoming connections consumed by the accept loop.
///
/// Implementations must be cancellation-safe: dropping a pending `accept()`
/// future must not leak resources.
#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub(super) trait AcceptListener: Send + Sync {
    async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

#[async_trait]
impl AcceptListener for TcpListener {
    async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        TcpListener::accept(self).await
    }

    fn local_addr(&self) -> io::Result<SocketAddr> { TcpListener::local_addr(self) }
}

#[derive(Debug)]
pub(super) struct AcceptLoopOptions<T> {
    pub preamble: PreambleHooks<T>,
    pub shutdown: CancellationToken,
    pub tracker: TaskTracker,
    pub backoff: BackoffConfig,
}

struct AcceptHandles<'a, T> {
    preamble: &'a PreambleHooks<T>,
    shutdown: &'a CancellationToken,
    tracker: &'a TaskTracker,
    backoff: &'a BackoffConfig,
}

#[derive(Default)]
pub(super) struct PreambleHooks<T> {
    pub on_success: Option<PreambleHandler<T>>,
    pub on_failure: Option<PreambleFailure>,
    pub timeout: Option<Duration>,
}

impl<T> Clone for PreambleHooks<T> {
    fn clone(&self) -> Self {
        Self {
            on_success: self.on_success.clone(),
            on_failure: self.on_failure.clone(),
            timeout: self.timeout,
        }
    }
}

impl<T> fmt::Debug for PreambleHooks<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreambleHooks")
            .field(
                "on_success",
                &self.on_success.as_ref().map(|_| "Some(<handler>)"),
            )
            .field(
                "on_failure",
                &self.on_failure.as_ref().map(|_| "Some(<failure>)"),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl<F, T, Ser, Ctx, E, Codec> WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>
where
    F: Fn() -> WireframeApp<Ser, Ctx, E, Codec> + Send + Sync + Clone + 'static,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    /// Run the server until a shutdown signal is received.
    ///
    /// Spawns the configured number of worker tasks and awaits Ctrl+C for shutdown.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), wireframe::server::ServerError> {
    /// let server =
    ///     WireframeServer::new(|| WireframeApp::default()).bind(([127, 0, 0, 1], 8080).into())?;
    /// server.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Attempting to run a server without binding fails to compile:
    ///
    /// Binding specifies the network address for the server to listen on.
    /// It is required so the server knows where to accept incoming connections.
    ///
    /// ```compile_fail
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// async fn try_run() {
    ///     WireframeServer::new(|| WireframeApp::default())
    ///         .run()
    ///         .await
    ///         .expect("unbound servers do not expose run()");
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the server was not bound to a listener.
    /// Accept failures are retried with exponential back-off and do not
    /// surface as errors.
    pub async fn run(self) -> Result<(), ServerError> {
        self.run_with_shutdown(async {
            let _ = signal::ctrl_c().await;
        })
        .await
    }

    /// Run the server until the `shutdown` future resolves.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::sync::oneshot;
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), wireframe::server::ServerError> {
    /// let server =
    ///     WireframeServer::new(|| WireframeApp::default()).bind(([127, 0, 0, 1], 0).into())?;
    ///
    /// let (tx, rx) = oneshot::channel::<()>();
    /// let handle = tokio::spawn(async move {
    ///     server
    ///         .run_with_shutdown(async {
    ///             let _ = rx.await;
    ///         })
    ///         .await
    /// });
    ///
    /// // Signal shutdown
    /// let _ = tx.send(());
    /// handle.await??;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Attempting to run a server without binding fails to compile:
    ///
    /// Binding specifies the network address for the server to listen on.
    /// It is required so the server knows where to accept incoming connections.
    ///
    /// ```compile_fail
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// async fn try_run_with_shutdown() {
    ///     WireframeServer::new(|| WireframeApp::default())
    ///         .run_with_shutdown(async {})
    ///         .await
    ///         .expect("unbound servers do not expose run_with_shutdown()");
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the server was not bound to a listener.
    /// Accept failures are retried with exponential back-off and do not
    /// surface as errors.
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus internally"
    )]
    pub async fn run_with_shutdown<S>(self, shutdown: S) -> Result<(), ServerError>
    where
        S: Future<Output = ()> + Send,
    {
        let WireframeServer {
            factory,
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            state: Bound { listener },
            backoff_config,
            preamble_timeout,
            ..
        } = self;
        let shutdown_token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let preamble = PreambleHooks {
            on_success: on_preamble_success,
            on_failure: on_preamble_failure,
            timeout: preamble_timeout,
        };

        for _ in 0..workers {
            let listener = Arc::clone(&listener);
            let factory = factory.clone();
            let preamble_hooks = preamble.clone();
            let token = shutdown_token.clone();
            let t = tracker.clone();
            tracker.spawn(accept_loop(
                listener,
                factory,
                AcceptLoopOptions {
                    preamble: preamble_hooks,
                    shutdown: token,
                    tracker: t,
                    backoff: backoff_config,
                },
            ));
        }

        // Signal readiness after all workers have been spawned.
        if let Some(tx) = ready_tx
            && tx.send(()).is_err()
        {
            warn!("Failed to send readiness signal: receiver dropped");
        }

        select! {
            () = shutdown => shutdown_token.cancel(),
            () = tracker.wait() => {},
        }

        tracker.close();
        tracker.wait().await;
        Ok(())
    }
}

/// Accepts incoming connections and spawns handler tasks.
///
/// The loop accepts connections from `listener`, creates a new
/// [`WireframeApp`] via `factory` for each one, and spawns a task to handle
/// the connection. Failures to accept a connection trigger an exponential
/// back-off governed by `backoff_config`. The loop terminates when `shutdown`
/// is cancelled, and all spawned tasks are tracked by `tracker` for graceful
/// shutdown.
///
/// # Parameters
///
/// - `listener`: Source of incoming TCP connections.
/// - `factory`: Creates a fresh [`WireframeApp`] for each connection.
/// - `preamble`: Preamble handlers and timeout configuration.
/// - `shutdown`: Signal used to stop the accept loop.
/// - `tracker`: Task tracker used for graceful shutdown.
/// - `backoff_config`: Controls exponential back-off behaviour.
///
/// # Type Parameters
///
/// - `F`: Factory function that creates [`WireframeApp`] instances.
/// - `T`: Preamble type for connection handshaking.
/// - `L`: Listener type implementing [`AcceptListener`].
///
/// # Examples
///
/// ```ignore
/// use std::sync::Arc;
///
/// use tokio_util::{sync::CancellationToken, task::TaskTracker};
/// use wireframe::{
///     app::WireframeApp,
///     // server::runtime::{AcceptListener, BackoffConfig, PreambleHooks, accept_loop},
/// };
///
/// async fn run<L: AcceptListener + Send + Sync + 'static>(listener: Arc<L>) {
///     let tracker = TaskTracker::new();
///     let token = CancellationToken::new();
///     accept_loop(
///         listener,
///         || WireframeApp::default(),
///         AcceptLoopOptions::<()> {
///             preamble: PreambleHooks::default(),
///             shutdown: token,
///             tracker,
///             backoff: BackoffConfig::default(),
///         },
///     )
///     .await;
/// }
/// ```
pub(super) async fn accept_loop<F, T, L, App>(
    listener: Arc<L>,
    factory: F,
    options: AcceptLoopOptions<T>,
) where
    F: Fn() -> App + Send + Sync + Clone + 'static,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
    App: ConnectionApp,
{
    let AcceptLoopOptions {
        preamble,
        shutdown,
        tracker,
        backoff,
    } = options;
    let backoff = backoff.normalised();
    debug_assert!(
        backoff.initial_delay <= backoff.max_delay,
        "BackoffConfig invariant violated: initial_delay > max_delay"
    );
    debug_assert!(
        backoff.initial_delay >= Duration::from_millis(1),
        "BackoffConfig invariant violated: initial_delay < 1ms"
    );
    let mut delay = backoff.initial_delay;
    let handles = AcceptHandles {
        preamble: &preamble,
        shutdown: &shutdown,
        tracker: &tracker,
        backoff: &backoff,
    };
    while let Some(next_delay) = accept_iteration(&listener, &factory, &handles, delay).await {
        delay = next_delay;
    }
}

#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! expands to modulus internally"
)]
async fn accept_iteration<F, T, L, App>(
    listener: &Arc<L>,
    factory: &F,
    handles: &AcceptHandles<'_, T>,
    delay: Duration,
) -> Option<Duration>
where
    F: Fn() -> App + Send + Sync + Clone + 'static,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
    App: ConnectionApp,
{
    select! {
        biased;

        () = handles.shutdown.cancelled() => None,
        res = listener.accept() => Some(match res {
            Ok((stream, _)) => {
                spawn_connection_task(
                    stream,
                    (*factory).clone(),
                    handles.preamble.clone(),
                    handles.tracker,
                );
                handles.backoff.initial_delay
            }
            Err(e) => {
                let local_addr = listener.local_addr().ok();
                warn!("accept error: error={e:?}, local_addr={local_addr:?}");
                sleep(delay).await;
                (delay * 2).min(handles.backoff.max_delay)
            }
        }),
    }
}
