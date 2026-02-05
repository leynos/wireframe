//! Shared helpers for the preamble integration tests.

use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
};

use bincode::error::DecodeError;
use futures::future::BoxFuture;
use tokio::{
    net::TcpStream,
    sync::oneshot,
    time::{Duration, timeout},
};
use wireframe::{app::WireframeApp, server::WireframeServer};

use crate::common::{TestResult, unused_listener};

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub(crate) struct HotlinePreamble {
    /// Should always be `b"TRTPHOTL"`.
    pub(crate) magic: [u8; 8],
    /// Minimum server version this client supports.
    pub(crate) min_version: u16,
    /// Client version.
    pub(crate) client_version: u16,
}

impl HotlinePreamble {
    pub(crate) const MAGIC: [u8; 8] = *b"TRTPHOTL";

    pub(crate) fn validate(&self) -> Result<(), DecodeError> {
        if self.magic != Self::MAGIC {
            return Err(DecodeError::Other("invalid hotline preamble"));
        }
        Ok(())
    }
}

/// Create a server configured with `HotlinePreamble` handlers.
pub(crate) fn server_with_handlers<F, S, E>(
    factory: F,
    success: S,
    failure: E,
) -> WireframeServer<F, HotlinePreamble>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    S: for<'a> Fn(&'a HotlinePreamble, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync
        + 'static,
    E: for<'a> Fn(&'a DecodeError, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync
        + 'static,
{
    WireframeServer::new(factory)
        .workers(1)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success(success)
        .on_preamble_decode_failure(failure)
}

/// Run the provided server while executing `block`.
pub(crate) async fn with_running_server<F, T, Fut, B>(
    server: WireframeServer<F, T>,
    block: B,
) -> TestResult
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: wireframe::preamble::Preamble,
    Fut: std::future::Future<Output = TestResult>,
    B: FnOnce(std::net::SocketAddr) -> Fut,
{
    let listener = unused_listener();
    let server = server.bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| Box::<dyn Error + Send + Sync>::from("server missing local addr"))?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    block(addr).await?;
    let _ = shutdown_tx.send(());
    let run_result = handle.await?;
    run_result?;
    Ok(())
}

/// Alternate preamble used to verify handler overrides.
///
/// # Examples
/// ```rust,ignore
/// let preamble = OtherPreamble(1);
/// assert_eq!(preamble.0, 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub(crate) struct OtherPreamble(pub(crate) u8);

/// Shared oneshot sender holder used by callbacks.
pub(crate) type Holder = Arc<Mutex<Option<oneshot::Sender<()>>>>;

/// Create a callback sender holder with its paired receiver.
///
/// # Examples
/// ```rust,ignore
/// let (holder, rx) = channel_holder();
/// assert!(holder.lock().unwrap().is_some());
/// drop(rx);
/// ```
pub(crate) fn channel_holder() -> (Holder, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    (Arc::new(Mutex::new(Some(tx))), rx)
}

/// Take the sender from a mutex, returning an IO error on poison.
///
/// # Examples
/// ```rust,ignore
/// use std::sync::Mutex;
///
/// let holder = Mutex::new(Some(1));
/// let value = take_sender_io(&holder).unwrap();
/// assert_eq!(value, Some(1));
/// ```
pub(crate) fn take_sender_io<T>(holder: &Mutex<Option<T>>) -> io::Result<Option<T>> {
    holder
        .lock()
        .map_err(|e| io::Error::other(format!("lock poisoned: {e}")))
        .map(|mut guard| guard.take())
}

/// Signal the holder if a sender is still available.
///
/// # Examples
/// ```rust,ignore
/// let (holder, _rx) = channel_holder();
/// notify_holder(&holder).unwrap();
/// ```
pub(crate) fn notify_holder(holder: &Holder) -> io::Result<()> {
    if let Some(tx) = take_sender_io(holder)? {
        let _ = tx.send(());
    }
    Ok(())
}

/// Await a oneshot receiver within the provided duration.
///
/// # Examples
/// ```rust,ignore
/// # use tokio::sync::oneshot;
/// # use tokio::time::Duration;
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// let (tx, rx) = oneshot::channel();
/// let _ = tx.send(42);
/// let value = recv_within(Duration::from_millis(50), rx).await?;
/// assert_eq!(value, 42);
/// # Ok(())
/// # }
/// ```
pub(crate) async fn recv_within<T>(duration: Duration, rx: oneshot::Receiver<T>) -> TestResult<T> {
    Ok(timeout(duration, rx).await??)
}

/// Build a success callback that signals through a shared holder.
///
/// # Examples
/// ```rust,ignore
/// let (holder, _rx) = channel_holder();
/// let callback = success_cb::<HotlinePreamble>(holder);
/// ```
pub(crate) fn success_cb<P>(
    holder: Arc<Mutex<Option<oneshot::Sender<()>>>>,
) -> impl for<'a> Fn(&'a P, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>> + Send + Sync + 'static
{
    move |_, _| {
        let holder = holder.clone();
        Box::pin(async move {
            if let Some(tx) = take_sender_io(&holder)? {
                let _ = tx.send(());
            }
            Ok(())
        })
    }
}

/// Build a failure callback that signals through a shared holder.
///
/// # Examples
/// ```rust,ignore
/// let (holder, _rx) = channel_holder();
/// let callback = failure_cb(holder);
/// ```
pub(crate) fn failure_cb(
    holder: Arc<Mutex<Option<oneshot::Sender<()>>>>,
) -> impl for<'a> Fn(&'a DecodeError, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
+ Send
+ Sync
+ 'static {
    move |_, _| {
        let holder = holder.clone();
        Box::pin(async move {
            if let Some(tx) = take_sender_io(&holder)? {
                let _ = tx.send(());
            }
            Ok(())
        })
    }
}
