//! Runtime control for [`WireframeServer`].

use std::{fmt, io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
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
    connection::spawn_connection_task,
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
    /// Delay used for the first retry after an `accept()` failure.
    pub initial_delay: Duration,
    /// Maximum back-off delay once retries have increased exponentially.
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
    /// Clamp delays to sane bounds and ensure `initial_delay <= max_delay`.
    ///
    /// This prevents accidental misconfiguration (for example, inverted or
    /// zero durations) before the values are used in the accept loop.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::server::runtime::BackoffConfig;
    ///
    /// let cfg = BackoffConfig {
    ///     initial_delay: Duration::from_millis(5),
    ///     max_delay: Duration::from_millis(1),
    /// };
    ///
    /// let normalised = cfg.normalised();
    /// assert_eq!(normalised.initial_delay, Duration::from_millis(1));
    /// assert_eq!(normalised.max_delay, Duration::from_millis(5));
    /// ```
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
        if ready_tx.is_some_and(|tx| tx.send(()).is_err()) {
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
pub(super) async fn accept_loop<F, T, L, Ser, Ctx, E, Codec>(
    listener: Arc<L>,
    factory: F,
    options: AcceptLoopOptions<T>,
) where
    F: Fn() -> WireframeApp<Ser, Ctx, E, Codec> + Send + Sync + Clone + 'static,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
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
async fn accept_iteration<F, T, L, Ser, Ctx, E, Codec>(
    listener: &Arc<L>,
    factory: &F,
    handles: &AcceptHandles<'_, T>,
    delay: Duration,
) -> Option<Duration>
where
    F: Fn() -> WireframeApp<Ser, Ctx, E, Codec> + Send + Sync + Clone + 'static,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
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
        let factory = move || -> WireframeApp {
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

        tracker.spawn(accept_loop(
            listener,
            factory,
            AcceptLoopOptions::<()> {
                preamble: PreambleHooks::default(),
                shutdown: token.clone(),
                tracker: tracker.clone(),
                backoff: BackoffConfig::default(),
            },
        ));

        token.cancel();
        tracker.close();

        let result = timeout(Duration::from_millis(100), tracker.wait()).await;
        assert!(result.is_ok());
    }

    /// Creates a mock listener that fails with exponential backoff tracking.
    fn setup_backoff_mock_listener(
        calls: &Arc<Mutex<Vec<Instant>>>,
        num_calls: usize,
    ) -> MockAcceptListener {
        let mut listener = MockAcceptListener::new();
        let call_log = Arc::clone(calls);
        listener
            .expect_accept()
            .returning(move || {
                let call_log = Arc::clone(&call_log);
                Box::pin(async move {
                    call_log.lock().expect("lock").push(Instant::now());
                    Err(io::Error::other("mock error"))
                })
            })
            .times(num_calls);
        listener
            .expect_local_addr()
            .returning(|| Ok("127.0.0.1:0".parse().expect("addr parse")))
            .times(num_calls);
        listener
    }

    /// Validates that recorded call intervals match expected backoff delays.
    fn assert_backoff_intervals(calls: &[Instant], expected: &[Duration]) {
        let intervals: Vec<_> = calls
            .windows(2)
            .map(|w| {
                let a = w.first().expect("window has first element");
                let b = w.get(1).expect("window has second element");
                b.checked_duration_since(*a)
                    .expect("instants should be monotonically increasing")
            })
            .collect();

        assert_eq!(
            intervals.len(),
            expected.len(),
            "interval count mismatch: got {}, expected {}",
            intervals.len(),
            expected.len()
        );

        for (interval, expected) in intervals.into_iter().zip(expected.iter()) {
            assert_eq!(interval, *expected);
        }
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn test_accept_loop_exponential_backoff_async(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let listener = Arc::new(setup_backoff_mock_listener(&calls, 4));
        let token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let backoff = BackoffConfig {
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        };

        tracker.spawn(accept_loop(
            listener,
            factory,
            AcceptLoopOptions::<()> {
                preamble: PreambleHooks::default(),
                shutdown: token.clone(),
                tracker: tracker.clone(),
                backoff,
            },
        ));

        yield_now().await;

        let first_call = {
            let calls = calls.lock().expect("lock");
            assert_eq!(calls.len(), 1);
            calls.first().copied().expect("call record missing")
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
        let first = calls.first().copied().expect("at least one call logged");
        assert_eq!(first, first_call);
        let expected = [
            Duration::from_millis(5),
            Duration::from_millis(10),
            Duration::from_millis(20),
        ];
        assert_backoff_intervals(&calls, &expected);
    }
}
