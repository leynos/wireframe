//! Shared example helpers for simple accept-or-shutdown server loops.

use tokio::{
    net::{TcpListener, TcpStream},
    signal,
};
use tracing::{error, info};

#[expect(
    clippy::cognitive_complexity,
    reason = "This helper only logs the two shutdown outcomes; tracing macros inflate the lint \
              score"
)]
fn log_shutdown_result(result: std::io::Result<()>, shutdown_message: &str) {
    if let Err(error) = result {
        error!("failed waiting for shutdown signal: {error}");
    } else {
        info!("{shutdown_message}");
    }
}

/// Accept the next client connection or stop when `ctrl_c` fires.
///
/// Returns `Ok(Some(stream))` when a client connects and `Ok(None)` after a
/// shutdown signal has been observed and logged.
///
/// # Errors
/// Returns any listener accept error.
#[expect(
    clippy::integer_division_remainder_used,
    reason = "`tokio::select!` macro expansion performs modulo internally"
)]
pub async fn accept_until_shutdown(
    listener: &TcpListener,
    shutdown_message: &str,
) -> std::io::Result<Option<TcpStream>> {
    tokio::select! {
        accept_result = listener.accept() => accept_result.map(|(stream, _)| Some(stream)),
        shutdown_result = signal::ctrl_c() => {
            log_shutdown_result(shutdown_result, shutdown_message);
            Ok(None)
        }
    }
}
