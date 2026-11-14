//! Connection task spawning for [`WireframeServer`].

use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use futures::FutureExt;
use log::{error, warn};
use tokio::{net::TcpStream, time::timeout};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    app::{Envelope, Packet, PreparedApp},
    codec::FrameCodec,
    connection::ActiveConnection,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    preamble::{Preamble, read_preamble},
    rewind_stream::RewindStream,
    serializer::Serializer,
    server::{PreambleFailure, PreambleHandler, runtime::PreambleHooks},
};

/// Accept-side resources handed to one spawned connection task.
///
/// The accept loop builds this value the moment `accept()` succeeds, so the
/// [`ActiveConnection`] guard is created before the task exists.
pub(super) struct ConnectionTask<T, Ser, Ctx, E, Codec>
where
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    /// Accepted socket.
    pub(super) stream: TcpStream,
    /// Shared prepared application.
    pub(super) app: Arc<PreparedApp<Ser, Ctx, E, Codec>>,
    /// Preamble callbacks configured on the server.
    pub(super) hooks: PreambleHooks<T>,
    /// Accept-side guard held for the whole task lifetime.
    pub(super) connection: ActiveConnection,
    /// Worker shutdown signal; cancelling it stops this connection.
    pub(super) shutdown: CancellationToken,
}

/// Spawn a task to process a single TCP connection, logging and discarding any panics.
///
/// `task.connection` is the accept-side [`ActiveConnection`] guard. It is moved
/// into the spawned future and released when that future ends, so the active
/// connection gauge covers preamble handling, application processing, teardown,
/// panic unwinding, and cancellation. Cancelling a future that never ran drops
/// the guard too.
pub(super) fn spawn_connection_task<T, Ser, Ctx, E, Codec>(
    task: ConnectionTask<T, Ser, Ctx, E, Codec>,
    tracker: &TaskTracker,
) where
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    tracker.spawn(async move {
        let peer_addr = match task.stream.peer_addr() {
            Ok(addr) => Some(addr),
            Err(e) => {
                warn!("Failed to retrieve peer address: error={e}");
                None
            }
        };
        // `process_stream` keeps the gauge held for every exit path, including
        // a panic caught by `catch_unwind` here and cancellation of this
        // future.
        let fut = std::panic::AssertUnwindSafe(process_stream(task, peer_addr)).catch_unwind();

        if let Err(panic) = fut.await {
            crate::metrics::inc_connection_panics();
            let panic_msg = crate::panic::format_panic(&panic);
            // Emit via both `log` and `tracing` for tests that capture either.
            error!("connection task panicked: panic={panic_msg}, peer_addr={peer_addr:?}");
            tracing::error!(panic = %panic_msg, ?peer_addr, "connection task panicked");
        }
    });
}

/// Complete the preamble handshake before handing bytes to the application.
///
/// The accept-side [`ActiveConnection`] guard travels with the task, so the
/// gauge covers preamble handling, application processing, teardown, panic
/// unwinding, and cancellation.
async fn process_stream<T, Ser, Ctx, E, Codec>(
    task: ConnectionTask<T, Ser, Ctx, E, Codec>,
    peer_addr: Option<SocketAddr>,
) where
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let ConnectionTask {
        mut stream,
        app,
        hooks,
        connection,
        shutdown,
    } = task;
    let _connection = connection;

    let Some(leftover) = complete_handshake(&mut stream, hooks, &shutdown, peer_addr).await else {
        return;
    };
    let rewind_stream = RewindStream::new(leftover, stream);
    handle_app_connection(&app, rewind_stream, shutdown).await;
}

/// Complete the preamble handshake, returning the bytes after the preamble.
///
/// Returns `None` when a stop arrives or the handshake fails, in which case the
/// task ends here and its gauge guard is released.
async fn complete_handshake<T>(
    stream: &mut TcpStream,
    hooks: PreambleHooks<T>,
    shutdown: &CancellationToken,
    peer_addr: Option<SocketAddr>,
) -> Option<Vec<u8>>
where
    T: Preamble,
{
    let (preamble, leftover) = read_handshake(stream, &hooks, shutdown, peer_addr).await?;
    let success = run_preamble_success(hooks.on_success.as_ref(), &preamble, stream, peer_addr);
    unless_stopping(shutdown, success)
        .await?
        .ok()
        .map(|()| leftover)
}

/// Read and validate the preamble, or report the failure to its hook.
async fn read_handshake<T>(
    stream: &mut TcpStream,
    hooks: &PreambleHooks<T>,
    shutdown: &CancellationToken,
    peer_addr: Option<SocketAddr>,
) -> Option<(T, Vec<u8>)>
where
    T: Preamble,
{
    let read = read_preamble_with_timeout::<T>(stream, hooks.timeout);
    match unless_stopping(shutdown, read).await? {
        Ok(result) => Some(result),
        Err(err) => {
            let report = run_preamble_failure(hooks.on_failure.as_ref(), err, stream, peer_addr);
            unless_stopping(shutdown, report).await;
            None
        }
    }
}

/// Await `work` unless the connection stops first.
///
/// Returns `None` once shutdown is requested, which abandons the work because
/// the connection is closing and there is no peer left to serve.
///
/// The handshake runs before the application observes the shutdown token, and a
/// peer may leave its preamble incomplete while the configured timeout is
/// `None`. Racing every handshake step here is what lets `stop()` release the
/// socket, the task and its gauge guard instead of waiting for that peer, and it
/// also keeps a slow preamble hook from delaying the drain.
async fn unless_stopping<W>(shutdown: &CancellationToken, work: W) -> Option<W::Output>
where
    W: std::future::Future,
{
    tokio::select! {
        biased;
        () = shutdown.cancelled() => None,
        output = work => Some(output),
    }
}

/// Run the shared prepared application on the already-handshaken stream.
async fn handle_app_connection<Ser, Ctx, E, Codec>(
    app: &PreparedApp<Ser, Ctx, E, Codec>,
    stream: RewindStream<TcpStream>,
    shutdown: CancellationToken,
) where
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    if let Err(error) = app.handle_connection_with_shutdown(stream, shutdown).await {
        warn!("connection task error: {error:?}");
    }
}

/// Construct the decode error used when a preamble read exceeds its deadline.
fn timeout_error() -> bincode::error::DecodeError {
    bincode::error::DecodeError::Io {
        inner: io::Error::new(io::ErrorKind::TimedOut, "preamble read timed out"),
        additional: 0,
    }
}

/// Read a preamble, applying the configured timeout without leaking the stream.
async fn read_preamble_with_timeout<T: Preamble>(
    stream: &mut TcpStream,
    preamble_timeout: Option<Duration>,
) -> Result<(T, Vec<u8>), bincode::error::DecodeError> {
    let Some(limit) = preamble_timeout else {
        return read_preamble::<_, T>(stream).await;
    };

    timeout(limit, read_preamble::<_, T>(stream))
        .await
        .map_err(|_| timeout_error())?
}

/// Invoke the success callback while the stream remains exclusively borrowed.
async fn run_preamble_success<T: Preamble>(
    handler: Option<&PreambleHandler<T>>,
    preamble: &T,
    stream: &mut TcpStream,
    peer_addr: Option<SocketAddr>,
) -> io::Result<()> {
    if let Some(preamble_handler) = handler
        && let Err(e) = preamble_handler(preamble, stream).await
    {
        error!("preamble handler error: error={e}, error_debug={e:?}, peer_addr={peer_addr:?}");
        crate::metrics::inc_preamble_rejection(
            crate::metrics::PreambleRejectionReason::SuccessHandlerError,
        );
        return Err(e);
    }
    Ok(())
}

/// Record bounded preamble decode rejection metrics and diagnostics.
fn record_preamble_rejection(err: &bincode::error::DecodeError, peer_addr: Option<SocketAddr>) {
    let reason = if matches!(
        err,
        bincode::error::DecodeError::LimitExceeded
            | bincode::error::DecodeError::Other(crate::preamble::PREAMBLE_TOO_LONG_ERROR)
    ) {
        crate::metrics::DecodeRejectionReason::LimitExceeded
    } else {
        crate::metrics::DecodeRejectionReason::DecodeError
    };
    crate::metrics::inc_decode_rejection(crate::metrics::DecodeRejectionPath::Preamble, reason);
    tracing::warn!(
        operation = "preamble_decode",
        rejection_category = reason.as_str(),
        ?peer_addr,
        error = ?err,
        "preamble rejected"
    );
}

/// Invoke the failure callback and preserve diagnostics for rejected peers.
async fn run_preamble_failure(
    handler: Option<&PreambleFailure>,
    err: bincode::error::DecodeError,
    stream: &mut TcpStream,
    peer_addr: Option<SocketAddr>,
) {
    record_preamble_rejection(&err, peer_addr);

    if let Some(failure_handler) = handler {
        if let Err(e) = failure_handler(&err, stream).await {
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

#[cfg(all(test, feature = "metrics"))]
#[path = "connection_spawner_metrics_tests.rs"]
mod metrics_tests;

#[cfg(all(test, feature = "metrics"))]
#[path = "connection_spawner_active_tests.rs"]
mod active_tests;

#[cfg(test)]
#[path = "connection_spawner_tests.rs"]
mod tests;
