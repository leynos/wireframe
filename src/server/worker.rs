//! Worker task and connection processing helpers for `WireframeServer`.

use std::sync::Arc;

use futures::FutureExt;
use tokio::{
    net::TcpListener,
    time::{Duration, sleep},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{PreambleCallback, PreambleErrorCallback};
use crate::{
    app::WireframeApp,
    preamble::{Preamble, read_preamble},
    rewind_stream::RewindStream,
};

/// Runs a worker task that accepts incoming TCP connections and processes them asynchronously.
///
/// Each accepted connection is handled in a separate task, with optional callbacks for preamble
/// decode success or failure. The worker listens for shutdown signals to terminate gracefully.
/// Accept errors are retried with exponential backoff.
pub async fn worker_task<F, T>(
    listener: Arc<TcpListener>,
    factory: F,
    on_success: Option<PreambleCallback<T>>,
    on_failure: Option<PreambleErrorCallback>,
    shutdown: CancellationToken,
    tracker: TaskTracker,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    // `Preamble` ensures `T` supports borrowed decoding.
    T: Preamble,
{
    let mut delay = Duration::from_millis(10);
    loop {
        tokio::select! {
            biased;

            () = shutdown.cancelled() => break,

            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    let success = on_success.clone();
                    let failure = on_failure.clone();
                    let factory = factory.clone();
                    let t = tracker.clone();
                    // Capture peer address for better error context
                    let peer_addr = stream.peer_addr().ok();
                    t.spawn(async move {
                        let fut = std::panic::AssertUnwindSafe(
                            process_stream(stream, factory, success, failure),
                        )
                        .catch_unwind();

                        if let Err(panic) = fut.await {
                            let panic_msg = panic
                                .downcast_ref::<&str>()
                                .copied()
                                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                                .unwrap_or("<non-string panic>");
                            tracing::error!(panic = %panic_msg, ?peer_addr, "connection task panicked");
                        }
                    });
                    delay = Duration::from_millis(10);
                }
                Err(e) => {
                    eprintln!("accept error: {e}");
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
            },
        }
    }
}

/// Processes an incoming TCP stream by decoding a preamble and dispatching the connection to a
/// `WireframeApp`.
///
/// Attempts to asynchronously decode a preamble of type `T` from the provided stream. If decoding
/// succeeds, invokes the optional success handler, wraps the stream to include any leftover bytes,
/// and passes it to a new `WireframeApp` instance for connection handling. If decoding fails,
/// invokes the optional failure handler and closes the connection.
///
/// # Type Parameters
///
/// - `F`: A factory closure that produces `WireframeApp` instances.
/// - `T`: The preamble type, which must support borrowed decoding via the `Preamble` trait.
///
/// # Examples
///
/// ```no_run
/// # use std::sync::Arc;
/// # use tokio::net::TcpStream;
/// # use wireframe::app::WireframeApp;
/// # async fn example() {
/// let stream: TcpStream = unimplemented!();
/// let factory = || WireframeApp::new();
/// // process_stream::<_, ()>(stream, factory, None, None).await;
/// # }
/// ```
pub(crate) async fn process_stream<F, T>(
    mut stream: tokio::net::TcpStream,
    factory: F,
    on_success: Option<PreambleCallback<T>>,
    on_failure: Option<PreambleErrorCallback>,
) where
    F: Fn() -> WireframeApp + Send + Sync + 'static,
    // `Preamble` ensures `T` supports borrowed decoding.
    T: Preamble,
{
    match read_preamble::<_, T>(&mut stream).await {
        Ok((preamble, leftover)) => {
            if let Some(handler) = on_success.as_ref()
                && let Err(e) = handler(&preamble, &mut stream).await
            {
                eprintln!("preamble callback error: {e}");
            }
            let stream = RewindStream::new(leftover, stream);
            // Hand the connection to the application for processing.
            // We already run `process_stream` inside a task, so spawning again
            // only adds overhead.
            let app = (factory)();
            app.handle_connection(stream).await;
        }
        Err(err) => {
            if let Some(handler) = on_failure.as_ref() {
                handler(&err);
            }
            // drop stream on failure
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};
    use tokio::{
        net::TcpListener,
        time::{Duration, timeout},
    };
    use tokio_util::{sync::CancellationToken, task::TaskTracker};

    use super::*;

    #[fixture]
    fn factory() -> impl Fn() -> WireframeApp + Send + Sync + Clone + 'static {
        || WireframeApp::default()
    }

    #[rstest]
    #[tokio::test]
    async fn test_worker_task_shutdown_signal(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());

        tracker.spawn(worker_task::<_, ()>(
            listener,
            factory,
            None,
            None,
            token.clone(),
            tracker.clone(),
        ));

        token.cancel();
        tracker.close();

        let result = timeout(Duration::from_millis(100), tracker.wait()).await;
        assert!(result.is_ok());
    }
}
