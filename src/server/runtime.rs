//! Runtime control for [`WireframeServer`].

use std::{io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use futures::Future;
use tokio::{
    net::{TcpListener, TcpStream},
    select,
    signal,
    time::{Duration, sleep},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    Bound,
    PreambleErrorHandler,
    PreambleHandler,
    ServerError,
    WireframeServer,
    connection::spawn_connection_task,
};
use crate::{app::WireframeApp, preamble::Preamble};

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

/// Configuration for exponential back-off timing in the accept loop.
///
/// Controls retry behaviour when `accept()` calls fail on the server's TCP listener.
/// The back-off starts at `initial_delay` and doubles on each failure, capped at `max_delay`.
///
/// # Default Values
/// - `initial_delay`: 10 milliseconds
/// - `max_delay`: 1 second
///
/// # Invariants
/// - `initial_delay` must not exceed `max_delay`
/// - `initial_delay` must be at least 1 millisecond
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackoffConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
        }
    }
}

impl BackoffConfig {
    #[must_use]
    pub fn normalised(mut self) -> Self {
        self.initial_delay = self.initial_delay.max(Duration::from_millis(1));
        self.max_delay = self.max_delay.max(Duration::from_millis(1));
        if self.initial_delay > self.max_delay {
            std::mem::swap(&mut self.initial_delay, &mut self.max_delay);
        }
        self
    }
}

impl<F, T> WireframeServer<F, T, Bound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
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
            ..
        } = self;
        let shutdown_token = CancellationToken::new();
        let tracker = TaskTracker::new();

        for _ in 0..workers {
            let listener = Arc::clone(&listener);
            let factory = factory.clone();
            let on_success = on_preamble_success.clone();
            let on_failure = on_preamble_failure.clone();
            let token = shutdown_token.clone();
            let t = tracker.clone();
            tracker.spawn(accept_loop(
                listener,
                factory,
                on_success,
                on_failure,
                token,
                t,
                backoff_config,
            ));
        }

        // Signal readiness after all workers have been spawned.
        if ready_tx.is_some_and(|tx| tx.send(()).is_err()) {
            tracing::warn!("Failed to send readiness signal: receiver dropped");
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
/// - `on_success`: Callback invoked after a successful preamble.
/// - `on_failure`: Callback invoked when the preamble fails.
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
/// use wireframe::{app::WireframeApp /*, server::runtime::{AcceptListener, BackoffConfig, accept_loop} */};
///
/// async fn run<L: AcceptListener + Send + Sync + 'static>(listener: Arc<L>) {
///     let tracker = TaskTracker::new();
///     let token = CancellationToken::new();
///     accept_loop::<_, (), _>(
///         listener,
///         || WireframeApp::default(),
///         None,
///         None,
///         token,
///         tracker,
///         BackoffConfig::default(),
///     )
///     .await;
/// }
/// ```
pub(super) async fn accept_loop<F, T, L>(
    listener: Arc<L>,
    factory: F,
    on_success: Option<PreambleHandler<T>>,
    on_failure: Option<PreambleErrorHandler>,
    shutdown: CancellationToken,
    tracker: TaskTracker,
    backoff_config: BackoffConfig,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
{
    debug_assert!(
        backoff_config.initial_delay <= backoff_config.max_delay,
        "BackoffConfig invariant violated: initial_delay > max_delay"
    );
    debug_assert!(
        backoff_config.initial_delay >= Duration::from_millis(1),
        "BackoffConfig invariant violated: initial_delay < 1ms"
    );
    let mut delay = backoff_config.initial_delay;
    loop {
        select! {
            biased;

            () = shutdown.cancelled() => break,

            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    spawn_connection_task(
                        stream,
                        factory.clone(),
                        on_success.clone(),
                        on_failure.clone(),
                        &tracker,
                    );
                    delay = backoff_config.initial_delay;
                }
                Err(e) => {
                    let local_addr = listener.local_addr().ok();
                    tracing::warn!(error = ?e, ?local_addr, "accept error");
                    sleep(delay).await;
                    delay = (delay * 2).min(backoff_config.max_delay);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use rstest::rstest;
    use tokio::{
        sync::oneshot,
        task::yield_now,
        time::{Duration, Instant, advance, timeout},
    };

    use super::{MockAcceptListener, *};
    use crate::server::test_util::{bind_server, factory, free_listener};

    #[rstest]
    #[tokio::test]
    async fn test_run_with_immediate_shutdown(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_listener: std::net::TcpListener,
    ) {
        let server = bind_server(factory, free_listener);
        let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;
        assert!(result.is_ok());
        assert!(result.expect("server did not finish in time").is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_server_graceful_shutdown_with_ctrl_c_simulation(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_listener: std::net::TcpListener,
    ) {
        let server = bind_server(factory, free_listener);
        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            server
                .run_with_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .expect("server run failed");
        });
        let _ = tx.send(());
        handle.await.expect("server join error");
    }

    #[rstest]
    #[tokio::test]
    async fn test_multiple_worker_creation(free_listener: std::net::TcpListener) {
        let call_count = Arc::new(AtomicUsize::new(0));
        let clone = call_count.clone();
        let factory = move || {
            clone.fetch_add(1, Ordering::SeqCst);
            WireframeApp::default()
        };
        let server = WireframeServer::new(factory)
            .workers(3)
            .bind_existing_listener(free_listener)
            .expect("Failed to bind");
        let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;
        assert!(result.is_ok());
        assert!(result.expect("server did not finish in time").is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_accept_loop_shutdown_signal(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let listener = Arc::new(
            TcpListener::bind("127.0.0.1:0")
                .await
                .expect("failed to bind test listener"),
        );

        tracker.spawn(accept_loop::<_, (), _>(
            listener,
            factory,
            None,
            None,
            token.clone(),
            tracker.clone(),
            BackoffConfig::default(),
        ));

        token.cancel();
        tracker.close();

        let result = timeout(Duration::from_millis(100), tracker.wait()).await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn test_accept_loop_exponential_backoff_async(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut listener = MockAcceptListener::new();
        let call_log = calls.clone();
        listener
            .expect_accept()
            .returning(move || {
                let call_log = Arc::clone(&call_log);
                Box::pin(async move {
                    call_log.lock().expect("lock").push(Instant::now());
                    Err(io::Error::other("mock error"))
                })
            })
            .times(4);
        listener
            .expect_local_addr()
            .returning(|| Ok("127.0.0.1:0".parse().expect("addr parse")))
            .times(4);
        let listener = Arc::new(listener);
        let token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let backoff = BackoffConfig {
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        };

        tracker.spawn(accept_loop::<_, (), _>(
            listener,
            factory,
            None,
            None,
            token.clone(),
            tracker.clone(),
            backoff,
        ));

        yield_now().await;

        let first_call = {
            let calls = calls.lock().expect("lock");
            assert_eq!(calls.len(), 1);
            calls[0]
        };

        for ms in [5, 10, 20] {
            advance(Duration::from_millis(ms)).await;
            yield_now().await;
        }

        token.cancel();
        advance(Duration::from_millis(20)).await;
        yield_now().await;
        tracker.close();
        tracker.wait().await;

        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0], first_call);
        let intervals: Vec<_> = calls.windows(2).map(|w| w[1] - w[0]).collect();
        let expected = [
            Duration::from_millis(5),
            Duration::from_millis(10),
            Duration::from_millis(20),
        ];
        for (interval, expected) in intervals.into_iter().zip(expected) {
            assert_eq!(interval, expected);
        }
    }
}
