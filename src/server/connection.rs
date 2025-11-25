//! Connection handling for [`WireframeServer`].

use std::{io, net::SocketAddr, time::Duration};

use futures::FutureExt;
use log::{error, warn};
use tokio::{net::TcpStream, time::timeout};
use tokio_util::task::TaskTracker;

use super::{PreambleFailure, PreambleHandler};
use crate::{
    app::WireframeApp,
    preamble::{Preamble, read_preamble},
    rewind_stream::RewindStream,
};

/// Spawn a task to process a single TCP connection, logging and discarding any panics.
pub(super) fn spawn_connection_task<F, T>(
    stream: TcpStream,
    factory: F,
    on_success: Option<PreambleHandler<T>>,
    on_failure: Option<PreambleFailure<T>>,
    preamble_timeout: Option<Duration>,
    tracker: &TaskTracker,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => Some(addr),
        Err(e) => {
            warn!("Failed to retrieve peer address: error={e}");
            None
        }
    };
    tracker.spawn(async move {
        let fut = std::panic::AssertUnwindSafe(process_stream(
            stream,
            peer_addr,
            factory,
            on_success,
            on_failure,
            preamble_timeout,
        ))
        .catch_unwind();

        if let Err(panic) = fut.await {
            crate::metrics::inc_connection_panics();
            let panic_msg = crate::panic::format_panic(&panic);
            // Emit via both `log` and `tracing` for tests that capture either.
            error!("connection task panicked: panic={panic_msg}, peer_addr={peer_addr:?}");
            tracing::error!(panic = %panic_msg, ?peer_addr, "connection task panicked");
        }
    });
}

async fn process_stream<F, T>(
    mut stream: TcpStream,
    peer_addr: Option<SocketAddr>,
    factory: F,
    on_success: Option<PreambleHandler<T>>,
    on_failure: Option<PreambleFailure<T>>,
    preamble_timeout: Option<Duration>,
) where
    F: Fn() -> WireframeApp + Send + Sync + 'static,
    T: Preamble,
{
    let preamble_result = match preamble_timeout {
        Some(limit) => match timeout(limit, read_preamble::<_, T>(&mut stream)).await {
            Ok(result) => result,
            Err(_) => Err(timeout_error()),
        },
        None => read_preamble::<_, T>(&mut stream).await,
    };

    match preamble_result {
        Ok((preamble, leftover)) => {
            if let Some(handler) = on_success.as_ref()
                && let Err(e) = handler(&preamble, &mut stream).await
            {
                error!(
                    "preamble handler error: error={e}, error_debug={e:?}, peer_addr={peer_addr:?}"
                );
            }
            let stream = RewindStream::new(leftover, stream);
            let app = (factory)();
            app.handle_connection(stream).await;
        }
        Err(err) => {
            if let Some(handler) = on_failure.as_ref() {
                if let Err(e) = handler(&err, &mut stream).await {
                    error!(
                        "preamble failure handler error: error={e}, error_debug={e:?}, \
                         peer_addr={peer_addr:?}"
                    );
                }
            } else {
                error!(
                    "preamble decode failed and no failure handler set: error={err:?}, \
                     peer_addr={peer_addr:?}"
                );
            }
        }
    }
}

fn timeout_error() -> bincode::error::DecodeError {
    bincode::error::DecodeError::Io {
        inner: io::Error::new(io::ErrorKind::TimedOut, "preamble read timed out"),
        additional: 0,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };
    use tokio_util::task::TaskTracker;
    use tracing_test::traced_test;

    use super::*;
    use crate::{
        app::WireframeApp,
        server::{
            WireframeServer,
            test_util::{factory, free_listener},
        },
    };

    /// Panics in connection handlers are logged and do not tear down the worker.
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn spawn_connection_task_logs_panic(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let app_factory = move || {
            factory()
                .on_connection_setup(|| async { panic!("boom") })
                .unwrap()
        };
        let tracker = TaskTracker::new();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener.local_addr");

        let handle = tokio::spawn({
            let tracker = tracker.clone();
            async move {
                let (stream, _) = listener.accept().await.expect("accept");
                spawn_connection_task::<_, ()>(stream, app_factory, None, None, None, &tracker);
                tracker.close();
                tracker.wait().await;
            }
        });

        let client = TcpStream::connect(addr).await.expect("connect");
        let peer_addr = client.local_addr().expect("client.local_addr");
        client.writable().await.expect("client.writable");
        client.try_write(&[0; 8]).expect("client.try_write");
        drop(client);

        handle.await.expect("join connection task driver");
        tokio::task::yield_now().await;

        logs_assert(|lines: &[&str]| {
            lines
                .iter()
                .find(|line| {
                    line.contains("connection task panicked")
                        && line.contains("panic=Any")
                        && line.contains(&format!("peer_addr=Some({peer_addr})"))
                })
                .map(|_| ())
                .ok_or_else(|| "panic log not found".to_string())
        });
    }

    /// Non-string panic payloads are formatted with `Debug` in logs.
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn spawn_connection_task_logs_non_string_panic(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let app_factory = move || {
            factory()
                .on_connection_setup(|| async { std::panic::panic_any(5_u32) })
                .expect("install panic setup callback")
        };
        let tracker = TaskTracker::new();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener.local_addr");

        let handle = tokio::spawn({
            let tracker = tracker.clone();
            async move {
                let (stream, _) = listener.accept().await.expect("accept");
                spawn_connection_task::<_, ()>(stream, app_factory, None, None, None, &tracker);
                tracker.close();
                tracker.wait().await;
            }
        });

        let client = TcpStream::connect(addr).await.expect("connect");
        let peer_addr = client.local_addr().expect("client.local_addr");
        client.writable().await.expect("client.writable");
        client.try_write(&[0; 8]).expect("client.try_write");
        drop(client);

        handle.await.expect("join connection task driver");
        tokio::task::yield_now().await;

        logs_assert(|lines: &[&str]| {
            lines
                .iter()
                .find(|line| {
                    line.contains("connection task panicked")
                        && line.contains("panic=Any")
                        && line.contains(&format!("peer_addr=Some({peer_addr})"))
                })
                .map(|_| ())
                .ok_or_else(|| "panic log not found".to_string())
        });
    }

    /// Ensure the server survives panicking connection tasks.
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn connection_panic_is_caught(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_listener: std::net::TcpListener,
    ) {
        let app_factory = move || {
            factory()
                .on_connection_setup(|| async { panic!("boom") })
                .expect("failed to install panic setup callback")
        };
        let server = WireframeServer::new(app_factory)
            .workers(1)
            .bind_existing_listener(free_listener)
            .expect("bind");
        let addr = server
            .local_addr()
            .expect("failed to retrieve server address");

        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            server
                .run_with_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .expect("server run failed");
        });

        let first = TcpStream::connect(addr)
            .await
            .expect("first connection should succeed");
        let peer_addr = first.local_addr().expect("first connection peer address");
        first.writable().await.expect("connection not writable");
        first
            .try_write(&[0; 8])
            .expect("failed to write dummy bytes");
        drop(first);
        TcpStream::connect(addr)
            .await
            .expect("second connection should succeed after panic");

        let _ = tx.send(());
        handle.await.expect("server join error");
        tokio::task::yield_now().await;

        logs_assert(|lines: &[&str]| {
            lines
                .iter()
                .find(|line| {
                    line.contains("connection task panicked")
                        && line.contains("panic=Any")
                        && line.contains(&format!("peer_addr=Some({peer_addr})"))
                })
                .map(|_| ())
                .ok_or_else(|| "panic log not found".to_string())
        });
    }
}
