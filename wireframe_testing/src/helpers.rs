//! Helper utilities for driving `WireframeApp` instances in tests.
//!
//! These functions spin up an application on an in-memory duplex stream and
//! collect the bytes written back by the app for assertions.

use bincode::config;
use bytes::BytesMut;
use rstest::fixture;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    frame::{FrameMetadata, FrameProcessor, LengthPrefixedProcessor},
    serializer::Serializer,
};

/// Create a default length-prefixed frame processor for tests.
#[fixture]
#[allow(
    unused_braces,
    reason = "Clippy is wrong here; this is not a redundant block"
)]
pub fn processor() -> LengthPrefixedProcessor { LengthPrefixedProcessor::default() }

pub trait TestSerializer:
    Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static
{
}

impl<T> TestSerializer for T where
    T: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static
{
}

const DEFAULT_CAPACITY: usize = 4096;

async fn drive_internal<F, Fut>(
    server_fn: F,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let (mut client, server) = duplex(capacity);
    let server_fut = server_fn(server);
    let client_fut = async {
        for frame in &frames {
            client.write_all(frame).await?;
        }
        client.shutdown().await?;

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await?;
        io::Result::Ok(buf)
    };

    let ((), buf) = tokio::join!(server_fut, client_fut);
    buf
}

pub async fn drive_with_frame<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frame_with_capacity(app, frame, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frame_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity(app, vec![frame], capacity).await
}

pub async fn drive_with_frames<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity(app, frames, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frames_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_internal(
        |server| async move { app.handle_connection(server).await },
        frames,
        capacity,
    )
    .await
}

pub async fn drive_with_frame_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frame_with_capacity_mut(app, frame, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frame_with_capacity_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity_mut(app, vec![frame], capacity).await
}

pub async fn drive_with_frames_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity_mut(app, frames, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frames_with_capacity_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_internal(
        |server| async { app.handle_connection(server).await },
        frames,
        capacity,
    )
    .await
}

pub async fn drive_with_bincode<M, S, C, E>(
    app: WireframeApp<S, C, E>,
    msg: M,
) -> io::Result<Vec<u8>>
where
    M: bincode::Encode,
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let bytes = bincode::encode_to_vec(msg, config::standard()).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bincode encode failed: {e}"),
        )
    })?;
    let mut framed = BytesMut::new();
    LengthPrefixedProcessor::default().encode(&bytes, &mut framed)?;
    drive_with_frame(app, framed.to_vec()).await
}

/// Run `app` with a single input `frame` using the default buffer capacity.
///
/// # Errors
///
/// Returns any I/O errors encountered while interacting with the in-memory
/// duplex stream.
pub async fn run_app_with_frame<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
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
pub async fn run_app_with_frame_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    run_app_with_frames_with_capacity(app, vec![frame], capacity).await
}

/// Run `app` with multiple input `frames` using the default buffer capacity.
///
/// # Errors
///
/// Returns any I/O errors encountered while interacting with the in-memory
/// duplex stream.
#[allow(dead_code)]
pub async fn run_app_with_frames<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
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
pub async fn run_app_with_frames_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
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

/// Run `app` against an empty duplex stream.
///
/// This helper drives the connection lifecycle without sending any frames,
/// ensuring setup and teardown callbacks execute.
///
/// # Panics
///
/// Panics if `handle_connection` fails.
pub async fn run_with_duplex_server<S, C, E>(app: WireframeApp<S, C, E>)
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let (_client, server) = duplex(64);
    app.handle_connection(server).await;
}
