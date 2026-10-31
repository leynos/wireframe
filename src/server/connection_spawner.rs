//! Connection task spawning for [`WireframeServer`].

use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use futures::FutureExt;
use log::{error, warn};
use tokio::{net::TcpStream, time::timeout};
use tokio_util::task::TaskTracker;

use crate::{
    app::{Envelope, Packet, PreparedApp},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    preamble::{Preamble, read_preamble},
    rewind_stream::RewindStream,
    serializer::Serializer,
    server::{PreambleFailure, PreambleHandler, runtime::PreambleHooks},
};

/// Spawn a task to process a single TCP connection, logging and discarding any panics.
pub(super) fn spawn_connection_task<T, Ser, Ctx, E, Codec>(
    stream: TcpStream,
    app: Arc<PreparedApp<Ser, Ctx, E, Codec>>,
    hooks: PreambleHooks<T>,
    tracker: &TaskTracker,
) where
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => Some(addr),
        Err(e) => {
            warn!("Failed to retrieve peer address: error={e}");
            None
        }
    };
    tracker.spawn(async move {
        let fut = std::panic::AssertUnwindSafe(process_stream(stream, peer_addr, app, hooks))
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

/// Complete the preamble handshake before handing bytes to the application.
async fn process_stream<T, Ser, Ctx, E, Codec>(
    mut stream: TcpStream,
    peer_addr: Option<SocketAddr>,
    app: Arc<PreparedApp<Ser, Ctx, E, Codec>>,
    hooks: PreambleHooks<T>,
) where
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let PreambleHooks {
        on_success,
        on_failure,
        timeout: preamble_timeout,
    } = hooks;

    let (preamble, leftover) =
        match read_preamble_with_timeout::<T>(&mut stream, preamble_timeout).await {
            Ok(result) => result,
            Err(err) => {
                run_preamble_failure(on_failure.as_ref(), err, &mut stream, peer_addr).await;
                return;
            }
        };

    run_preamble_success(on_success.as_ref(), &preamble, &mut stream, peer_addr).await;
    let rewind_stream = RewindStream::new(leftover, stream);
    handle_app_connection(&app, rewind_stream).await;
}

/// Run the shared prepared application on the already-handshaken stream.
async fn handle_app_connection<Ser, Ctx, E, Codec>(
    app: &PreparedApp<Ser, Ctx, E, Codec>,
    stream: RewindStream<TcpStream>,
) where
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    if let Err(error) = app.handle_connection_result(stream).await {
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
) {
    if let Some(preamble_handler) = handler
        && let Err(e) = preamble_handler(preamble, stream).await
    {
        error!("preamble handler error: error={e}, error_debug={e:?}, peer_addr={peer_addr:?}");
    }
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

#[cfg(test)]
#[path = "connection_spawner_tests.rs"]
mod tests;
