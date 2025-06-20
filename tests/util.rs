use tokio::io::{self, AsyncReadExt, AsyncWriteExt, duplex};
use wireframe::app::WireframeApp;

/// Feed a single frame into `app` and collect the response bytes.
///
/// # Errors
///
/// Propagates I/O errors from the in-memory connection.
///
/// # Panics
///
/// Panics if the spawned task running the application panics.
/// Optional duplex buffer capacity for `run_app_with_frame`.
const DEFAULT_CAPACITY: usize = 4096;

/// Run `app` with a single input `frame` using the default buffer capacity.
///
/// # Errors
///
/// Returns any I/O errors encountered while interacting with the in-memory
/// duplex stream.
pub async fn run_app_with_frame(app: WireframeApp, frame: Vec<u8>) -> io::Result<Vec<u8>> {
    run_app_with_frame_with_capacity(app, frame, DEFAULT_CAPACITY).await
}

/// Drive `app` with a single frame using a duplex buffer of `capacity` bytes.
///
/// # Errors
///
/// Propagates any I/O errors from the in-memory connection.
///
/// # Panics
///
/// Panics if the spawned task running the application panics.
pub async fn run_app_with_frame_with_capacity(
    app: WireframeApp,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>> {
    run_app_with_frames_with_capacity(app, vec![frame], capacity).await
}

/// Run `app` with multiple input `frames` using the default buffer capacity.
///
/// # Errors
///
/// Returns any I/O errors encountered while interacting with the in-memory
/// duplex stream.
pub async fn run_app_with_frames(app: WireframeApp, frames: Vec<Vec<u8>>) -> io::Result<Vec<u8>> {
    run_app_with_frames_with_capacity(app, frames, DEFAULT_CAPACITY).await
}

/// Drive `app` with multiple frames using a duplex buffer of `capacity` bytes.
///
/// # Errors
///
/// Propagates any I/O errors from the in-memory connection.
///
/// # Panics
///
/// Panics if the spawned task running the application panics.
pub async fn run_app_with_frames_with_capacity(
    app: WireframeApp,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>> {
    let (mut client, server) = duplex(capacity);
    let server_task = tokio::spawn(async move {
        app.handle_connection(server).await;
    });

    for frame in &frames {
        client.write_all(frame).await?;
    }
    client.shutdown().await?;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;

    server_task.await.unwrap();
    Ok(buf)
}

/// Convenience for constructing a default length-prefixed processor.
#[must_use]
pub fn default_processor() -> wireframe::frame::LengthPrefixedProcessor {
    wireframe::frame::LengthPrefixedProcessor::default()
}
