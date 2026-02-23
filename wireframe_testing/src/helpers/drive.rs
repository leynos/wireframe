//! Frame-oriented in-memory driving helpers.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use wireframe::app::{Packet, WireframeApp};

use super::{DEFAULT_CAPACITY, TestSerializer};

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
/// use tokio::io::{AsyncWriteExt, DuplexStream};
/// use wireframe_testing::helpers::drive::drive_internal;
///
/// async fn echo(mut server: DuplexStream) { let _ = server.write_all(&[1, 2]).await; }
///
/// # async fn demo() -> std::io::Result<()> {
/// let bytes = drive_internal(echo, vec![vec![0]], 64).await?;
/// assert_eq!(bytes, [1, 2]);
/// # Ok(())
/// # }
/// ```
pub(super) async fn drive_internal<F, Fut>(
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
                let panic_msg = wireframe::panic::format_panic(&panic);
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("server task failed: {panic_msg}"),
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
/// # use wireframe_testing::drive_with_frame;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> std::io::Result<()> {
/// let app = WireframeApp::new().expect("failed to initialize app");
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
    /// # use wireframe_testing::drive_with_frame_with_capacity;
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> std::io::Result<()> {
    /// let app = WireframeApp::new().expect("failed to initialize app");
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
    /// # use wireframe_testing::drive_with_frames;
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> std::io::Result<()> {
    /// let app = WireframeApp::new().expect("failed to initialize app");
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
/// # use wireframe_testing::drive_with_frames_with_capacity;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> std::io::Result<()> {
/// let app = WireframeApp::new().expect("failed to initialize app");
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
    /// # use wireframe_testing::drive_with_frame_mut;
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> std::io::Result<()> {
    /// let mut app = WireframeApp::new().expect("failed to initialize app");
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
    /// # use wireframe_testing::drive_with_frame_with_capacity_mut;
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> std::io::Result<()> {
    /// let mut app = WireframeApp::new().expect("failed to initialize app");
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
    /// # use wireframe_testing::drive_with_frames_mut;
    /// # use wireframe::app::WireframeApp;
    /// # async fn demo() -> std::io::Result<()> {
    /// let mut app = WireframeApp::new().expect("failed to initialize app");
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
/// # use wireframe_testing::drive_with_frames_with_capacity_mut;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> std::io::Result<()> {
/// let mut app = WireframeApp::new().expect("failed to initialize app");
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
