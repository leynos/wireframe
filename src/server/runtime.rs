//! Runtime control for [`WireframeServer`].

use std::sync::Arc;

use futures::Future;
use tokio::{
    net::TcpListener,
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

///
///
///
/// Configuration for exponential backoff timing in the accept loop.
///
/// Controls retry behavior when `accept()` calls fail on the server's TCP listener.
/// The backoff starts at `initial_delay` and doubles on each failure, capped at `max_delay`.
///
/// # Default Values
/// - `initial_delay`: 10 milliseconds
/// - `max_delay`: 1 second
///
/// # Invariants
/// - `initial_delay` must not exceed `max_delay`
/// - `initial_delay` must be at least 1 millisecond
#[derive(Clone, Copy, Debug)]
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
    /// # Errors
    ///
    /// Returns a [`ServerError`] if runtime initialisation fails.
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
    /// # Errors
    ///
    /// Returns a [`ServerError`] if runtime initialisation fails.
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
            ..
        } = self;

        if let Some(tx) = ready_tx
            && tx.send(()).is_err()
        {
            tracing::warn!("Failed to send readiness signal: receiver dropped");
        }

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

        select! {
            () = shutdown => shutdown_token.cancel(),
            () = tracker.wait() => {},
        }

        tracker.close();
        tracker.wait().await;
        Ok(())
    }
}

pub(super) async fn accept_loop<F, T>(
    listener: Arc<TcpListener>,
    factory: F,
    on_success: Option<PreambleHandler<T>>,
    on_failure: Option<PreambleErrorHandler>,
    shutdown: CancellationToken,
    tracker: TaskTracker,
    backoff_config: BackoffConfig,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
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
        atomic::{AtomicUsize, Ordering},
    };

    use rstest::rstest;
    use tokio::{
        sync::oneshot,
        time::{Duration, timeout},
    };

    use super::*;
    use crate::server::test_util::{bind_server, factory, free_port};

    #[rstest]
    #[tokio::test]
    async fn test_run_with_immediate_shutdown(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let server = bind_server(factory, free_port);
        let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_server_graceful_shutdown_with_ctrl_c_simulation(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let server = bind_server(factory, free_port);
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
    async fn test_multiple_worker_creation(free_port: std::net::SocketAddr) {
        let call_count = Arc::new(AtomicUsize::new(0));
        let clone = call_count.clone();
        let factory = move || {
            clone.fetch_add(1, Ordering::SeqCst);
            WireframeApp::default()
        };
        let server = WireframeServer::new(factory)
            .workers(3)
            .bind(free_port)
            .expect("Failed to bind");
        let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
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

        tracker.spawn(accept_loop::<_, ()>(
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
}
