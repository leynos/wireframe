//! Runtime-level helpers for running apps against in-memory streams.

use std::io;

use tokio::io::duplex;
use wireframe::app::{Packet, WireframeApp};

use super::{EMPTY_SERVER_CAPACITY, MAX_CAPACITY, TestSerializer, drive::drive_internal};

/// Run `app` with input `frames` using an optional duplex buffer `capacity`.
///
/// When `capacity` is `None`, a buffer of `DEFAULT_CAPACITY` bytes is used.
/// Frames are written to the client side in order and the bytes emitted by the
/// server are collected for inspection.
///
/// # Errors
///
/// Returns an error if `capacity` is zero or exceeds `MAX_CAPACITY`. Any panic
/// in the application task or I/O error on the duplex stream is also surfaced
/// as an error.
///
/// ```rust
/// # use wireframe_testing::run_app;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> std::io::Result<()> {
/// let app = WireframeApp::new().expect("failed to initialize app");
/// let out = run_app(app, vec![vec![1]], None).await?;
/// # let _ = out;
/// # Ok(())
/// # }
/// ```
pub async fn run_app<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: Option<usize>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let capacity = capacity.unwrap_or(super::DEFAULT_CAPACITY);
    if capacity == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capacity must be greater than zero",
        ));
    }
    if capacity > MAX_CAPACITY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("capacity must not exceed {MAX_CAPACITY} bytes"),
        ));
    }

    drive_internal(
        |server| async move { app.handle_connection(server).await },
        frames,
        capacity,
    )
    .await
}

/// Run `app` against an empty duplex stream.
///
/// This helper drives the connection lifecycle without sending any frames,
/// ensuring setup and teardown callbacks execute.
///
/// # Panics
///
/// Panics if `handle_connection` fails.
///
/// ```rust
/// # use wireframe_testing::run_with_duplex_server;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() {
/// let app = WireframeApp::new().expect("failed to initialize app");
/// run_with_duplex_server(app).await;
/// # }
/// ```
pub async fn run_with_duplex_server<S, C, E>(app: WireframeApp<S, C, E>)
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let (_, server) = duplex(EMPTY_SERVER_CAPACITY); // discard client half
    app.handle_connection(server).await;
}
