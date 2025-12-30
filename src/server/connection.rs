//! Connection handling for [`WireframeServer`].

use std::{future::Future, io, net::SocketAddr, time::Duration};

use futures::FutureExt;
use log::{error, warn};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    time::timeout,
};
use tokio_util::task::TaskTracker;

use crate::{
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    frame::FrameMetadata,
    preamble::{Preamble, read_preamble},
    rewind_stream::RewindStream,
    serializer::Serializer,
    server::{PreambleFailure, PreambleHandler, runtime::PreambleHooks},
};

pub(super) trait ConnectionApp: Send + Sync + 'static {
    fn handle_connection_result<W>(&self, stream: W) -> impl Future<Output = io::Result<()>> + Send
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static;
}

impl<S, C, E, Codec> ConnectionApp for WireframeApp<S, C, E, Codec>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
{
    fn handle_connection_result<W>(&self, stream: W) -> impl Future<Output = io::Result<()>> + Send
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        WireframeApp::handle_connection_result(self, stream)
    }
}

/// Spawn a task to process a single TCP connection, logging and discarding any panics.
pub(super) fn spawn_connection_task<F, T, App>(
    stream: TcpStream,
    factory: F,
    hooks: PreambleHooks<T>,
    tracker: &TaskTracker,
) where
    F: Fn() -> App + Send + Sync + Clone + 'static,
    T: Preamble,
    App: ConnectionApp,
{
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => Some(addr),
        Err(e) => {
            warn!("Failed to retrieve peer address: error={e}");
            None
        }
    };
    tracker.spawn(async move {
        let fut = std::panic::AssertUnwindSafe(process_stream(stream, peer_addr, factory, hooks))
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

async fn process_stream<F, T, App>(
    mut stream: TcpStream,
    peer_addr: Option<SocketAddr>,
    factory: F,
    hooks: PreambleHooks<T>,
) where
    F: Fn() -> App + Send + Sync + 'static,
    T: Preamble,
    App: ConnectionApp,
{
    let PreambleHooks {
        on_success,
        on_failure,
        timeout: preamble_timeout,
    } = hooks;

    match read_preamble_with_timeout::<T>(&mut stream, preamble_timeout).await {
        Ok((preamble, leftover)) => {
            run_preamble_success(on_success.as_ref(), &preamble, &mut stream, peer_addr).await;
            let stream = RewindStream::new(leftover, stream);
            if let Err(e) = (factory)().handle_connection_result(stream).await {
                warn!("connection task error: {e:?}");
            }
        }
        Err(err) => {
            run_preamble_failure(on_failure.as_ref(), err, &mut stream, peer_addr).await;
        }
    }
}

fn timeout_error() -> bincode::error::DecodeError {
    bincode::error::DecodeError::Io {
        inner: io::Error::new(io::ErrorKind::TimedOut, "preamble read timed out"),
        additional: 0,
    }
}

async fn read_preamble_with_timeout<T: Preamble>(
    stream: &mut TcpStream,
    preamble_timeout: Option<Duration>,
) -> Result<(T, Vec<u8>), bincode::error::DecodeError> {
    match preamble_timeout {
        Some(limit) => match timeout(limit, read_preamble::<_, T>(stream)).await {
            Ok(result) => result,
            Err(_) => Err(timeout_error()),
        },
        None => read_preamble::<_, T>(stream).await,
    }
}

async fn run_preamble_success<T: Preamble>(
    handler: Option<&PreambleHandler<T>>,
    preamble: &T,
    stream: &mut TcpStream,
    peer_addr: Option<SocketAddr>,
) {
    if let Some(handler) = handler
        && let Err(e) = handler(preamble, stream).await
    {
        error!("preamble handler error: error={e}, error_debug={e:?}, peer_addr={peer_addr:?}");
    }
}

async fn run_preamble_failure(
    handler: Option<&PreambleFailure>,
    err: bincode::error::DecodeError,
    stream: &mut TcpStream,
    peer_addr: Option<SocketAddr>,
) {
    if let Some(handler) = handler {
        if let Err(e) = handler(&err, stream).await {
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
                .expect("failed to install panic setup callback")
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
                spawn_connection_task(
                    stream,
                    app_factory,
                    PreambleHooks::<()>::default(),
                    &tracker,
                );
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
                spawn_connection_task(
                    stream,
                    app_factory,
                    PreambleHooks::<()>::default(),
                    &tracker,
                );
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
