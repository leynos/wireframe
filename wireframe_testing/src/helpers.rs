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

/// Run `server_fn` against a duplex stream, writing each `frame` to the client
/// half and returning the bytes produced by the server.
///
/// The server function receives the server half of a `tokio::io::duplex`
/// connection. Every provided frame is written to the client side in order and
/// the collected output is returned once the server task completes. If the
/// server panics, the panic message is surfaced as an `io::Error` beginning
/// with `"server task failed"`.
///
/// ```rust
/// use tokio::io::{self, AsyncWriteExt, DuplexStream};
/// use wireframe_testing::helpers::drive_internal;
///
/// async fn echo(mut server: DuplexStream) { let _ = server.write_all(&[1, 2]).await; }
///
/// # async fn demo() -> io::Result<()> {
/// let bytes = drive_internal(echo, vec![vec![0]], 64).await?;
/// assert_eq!(bytes, [1, 2]);
/// # Ok(())
/// # }
/// ```
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

    let server_fut = async {
        use futures::FutureExt as _;
        let result = std::panic::AssertUnwindSafe(server_fn(server))
            .catch_unwind()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(panic) => {
                let msg = wireframe::panic::format_panic(panic);
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("server task failed: {msg}"),
                ))
            }
        }
    };

    let client_fut = async {
        for frame in &frames {
            client.write_all(frame).await?;
        }
        client.shutdown().await?;

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await?;
        io::Result::Ok(buf)
    };

    let ((), buf) = tokio::try_join!(server_fut, client_fut)?;
    Ok(buf)
}

const DEFAULT_CAPACITY: usize = 4096;
const MAX_CAPACITY: usize = 1024 * 1024 * 10; // 10MB limit

macro_rules! forward_default {
    (
        $(#[$docs:meta])* $vis:vis fn $name:ident(
            $app:ident : $app_ty:ty,
            $arg:ident : $arg_ty:ty
        ) -> $ret:ty
        => $inner:ident($app_expr:ident, $arg_expr:expr)
    ) => {
        $(#[$docs])*
        $vis async fn $name<S, C, E>(
            $app: $app_ty,
            $arg: $arg_ty,
        ) -> $ret
        where
            S: TestSerializer,
            C: Send + 'static,
            E: Packet,
        {
            $inner($app_expr, $arg_expr, DEFAULT_CAPACITY).await
        }
    };
}

macro_rules! forward_with_capacity {
    (
        $(#[$docs:meta])* $vis:vis fn $name:ident(
            $app:ident : $app_ty:ty,
            $arg:ident : $arg_ty:ty,
            capacity: usize
        ) -> $ret:ty
        => $inner:ident($app_expr:ident, $arg_expr:expr, capacity)
    ) => {
        $(#[$docs])*
        $vis async fn $name<S, C, E>(
            $app: $app_ty,
            $arg: $arg_ty,
            capacity: usize,
        ) -> $ret
        where
            S: TestSerializer,
            C: Send + 'static,
            E: Packet,
        {
            $inner($app_expr, $arg_expr, capacity).await
        }
    };
}

/// Drive `app` with a single length-prefixed `frame` and return the bytes
/// produced by the server.
///
/// The app runs on an in-memory duplex stream so tests need not open real
/// sockets.
///
/// # Errors
///
/// Returns any I/O errors encountered while interacting with the in-memory
/// duplex stream.
///
/// ```rust
/// # use wireframe_testing::{drive_with_frame, processor};
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> tokio::io::Result<()> {
/// let app = WireframeApp::new().frame_processor(processor()).unwrap();
/// let bytes = drive_with_frame(app, vec![1, 2, 3]).await?;
/// # Ok(())
/// # }
/// ```
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

forward_with_capacity! {
    /// Drive `app` with a single frame using a duplex buffer of `capacity` bytes.
    ///
    /// Adjusting the buffer size helps exercise edge cases such as small channels.
    ///
    /// ```rust
    /// # use wireframe_testing::{drive_with_frame_with_capacity, processor};
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> tokio::io::Result<()> {
    /// let app = WireframeApp::new().frame_processor(processor()).unwrap();
    /// let bytes = drive_with_frame_with_capacity(app, vec![0], 512).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn drive_with_frame_with_capacity(app: WireframeApp<S, C, E>, frame: Vec<u8>, capacity: usize) -> io::Result<Vec<u8>>
    => drive_with_frames_with_capacity(app, vec![frame], capacity)
}

forward_default! {
    /// Drive `app` with a sequence of frames using the default buffer size.
    ///
    /// Each frame is written to the duplex stream in order.
    ///
    /// ```rust
    /// # use wireframe_testing::{drive_with_frames, processor};
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> tokio::io::Result<()> {
    /// let app = WireframeApp::new().frame_processor(processor()).unwrap();
    /// let out = drive_with_frames(app, vec![vec![1], vec![2]]).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn drive_with_frames(app: WireframeApp<S, C, E>, frames: Vec<Vec<u8>>) -> io::Result<Vec<u8>>
    => drive_with_frames_with_capacity(app, frames)
}

/// Drive `app` with multiple frames using a duplex buffer of `capacity` bytes.
///
/// This variant exposes the buffer size for fine-grained control in tests.
///
/// ```rust
/// # use wireframe_testing::{drive_with_frames_with_capacity, processor};
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> tokio::io::Result<()> {
/// let app = WireframeApp::new().frame_processor(processor()).unwrap();
/// let out = drive_with_frames_with_capacity(app, vec![vec![1], vec![2]], 1024).await?;
/// # Ok(())
/// # }
/// ```
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

forward_default! {
    /// Feed a single frame into a mutable `app`, allowing the instance to be reused
    /// across calls.
    ///
    /// ```rust
    /// # use wireframe_testing::{drive_with_frame_mut, processor};
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> tokio::io::Result<()> {
    /// let mut app = WireframeApp::new().frame_processor(processor()).unwrap();
    /// let bytes = drive_with_frame_mut(&mut app, vec![1]).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn drive_with_frame_mut(app: &mut WireframeApp<S, C, E>, frame: Vec<u8>) -> io::Result<Vec<u8>>
    => drive_with_frame_with_capacity_mut(app, frame)
}

forward_with_capacity! {
    /// Feed a single frame into `app` using a duplex buffer of `capacity` bytes.
    ///
    /// ```rust
    /// # use wireframe_testing::{drive_with_frame_with_capacity_mut, processor};
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> tokio::io::Result<()> {
    /// let mut app = WireframeApp::new().frame_processor(processor()).unwrap();
    /// let bytes = drive_with_frame_with_capacity_mut(&mut app, vec![1], 256).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn drive_with_frame_with_capacity_mut(app: &mut WireframeApp<S, C, E>, frame: Vec<u8>, capacity: usize) -> io::Result<Vec<u8>>
    => drive_with_frames_with_capacity_mut(app, vec![frame], capacity)
}

forward_default! {
    /// Feed multiple frames into a mutable `app`.
    ///
    /// ```rust
    /// # use wireframe_testing::{drive_with_frames_mut, processor};
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> tokio::io::Result<()> {
    /// let mut app = WireframeApp::new().frame_processor(processor()).unwrap();
    /// let out = drive_with_frames_mut(&mut app, vec![vec![1], vec![2]]).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn drive_with_frames_mut(app: &mut WireframeApp<S, C, E>, frames: Vec<Vec<u8>>) -> io::Result<Vec<u8>>
    => drive_with_frames_with_capacity_mut(app, frames)
}

/// Feed multiple frames into `app` with a duplex buffer of `capacity` bytes.
///
/// ```rust
/// # use wireframe_testing::{drive_with_frames_with_capacity_mut, processor};
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> tokio::io::Result<()> {
/// let mut app = WireframeApp::new().frame_processor(processor()).unwrap();
/// let out = drive_with_frames_with_capacity_mut(&mut app, vec![vec![1], vec![2]], 64).await?;
/// # Ok(())
/// # }
/// ```
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

/// Encode `msg` using bincode, frame it and drive `app`.
///
/// ```rust
/// # use wireframe_testing::{drive_with_bincode, processor};
/// # use wireframe::app::WireframeApp;
/// #[derive(bincode::Encode)]
/// struct Ping(u8);
/// # async fn demo() -> tokio::io::Result<()> {
/// let app = WireframeApp::new().frame_processor(processor()).unwrap();
/// let bytes = drive_with_bincode(app, Ping(1)).await?;
/// # Ok(())
/// # }
/// ```
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

/// Run `app` with input `frames` using an optional duplex buffer `capacity`.
///
/// When `capacity` is `None`, a buffer of [`DEFAULT_CAPACITY`] bytes is used.
/// Frames are written to the client side in order and the bytes emitted by the
/// server are collected for inspection.
///
/// # Errors
///
/// Returns an error if `capacity` is zero or exceeds [`MAX_CAPACITY`]. Any
/// panic in the application task or I/O error on the duplex stream is also
/// surfaced as an error.
///
/// ```rust
/// # use wireframe_testing::{processor, run_app};
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> tokio::io::Result<()> {
/// let app = WireframeApp::new().frame_processor(processor()).unwrap();
/// let out = run_app(app, vec![vec![1]], None).await?;
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
    let capacity = capacity.unwrap_or(DEFAULT_CAPACITY);
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

    let (mut client, server) = duplex(capacity);
    let server_task = tokio::spawn(async move { app.handle_connection(server).await });

    for frame in &frames {
        client.write_all(frame).await?;
    }
    client.shutdown().await?;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;

    if let Err(e) = server_task.await {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("server task failed: {e}"),
        ));
    }

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
///
/// ```rust
/// # use wireframe_testing::{run_with_duplex_server, processor};
/// # use wireframe::app::WireframeApp;
/// # async fn demo() {
/// let app = WireframeApp::new()
///     .frame_processor(processor())
///     .unwrap();
/// run_with_duplex_server(app).await;
/// }
/// ```
pub async fn run_with_duplex_server<S, C, E>(app: WireframeApp<S, C, E>)
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let (_client, server) = duplex(64);
    app.handle_connection(server).await;
}

/// Await the provided future and panic with context on failure.
///
/// In debug builds, the generated message includes the call site for easier
/// troubleshooting.
#[macro_export]
macro_rules! push_expect {
    ($fut:expr) => {{
        $fut.await
            .expect(concat!("push failed at ", file!(), ":", line!()))
    }};
    ($fut:expr, $msg:expr) => {{
        let m = ::std::format!("{msg} at {}:{}", file!(), line!(), msg = $msg);
        $fut.await.expect(&m)
    }};
}

/// Await the provided future and panic with context on failure.
///
/// In debug builds, the generated message includes the call site for easier
/// troubleshooting.
#[macro_export]
macro_rules! recv_expect {
    ($fut:expr) => {{
        $fut.await
            .expect(concat!("recv failed at ", file!(), ":", line!()))
    }};
    ($fut:expr, $msg:expr) => {{
        let m = ::std::format!("{msg} at {}:{}", file!(), line!(), msg = $msg);
        $fut.await.expect(&m)
    }};
}
