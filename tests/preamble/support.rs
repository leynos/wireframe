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

#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub(crate) struct OtherPreamble(pub(crate) u8);

pub(crate) type Holder = Arc<Mutex<Option<oneshot::Sender<()>>>>;

pub(crate) fn channel_holder() -> (Holder, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    (Arc::new(Mutex::new(Some(tx))), rx)
}

pub(crate) fn take_sender_io<T>(holder: &Mutex<Option<T>>) -> io::Result<Option<T>> {
    holder
        .lock()
        .map_err(|e| io::Error::other(format!("lock poisoned: {e}")))
        .map(|mut guard| guard.take())
}

pub(crate) async fn recv_within<T>(duration: Duration, rx: oneshot::Receiver<T>) -> TestResult<T> {
    Ok(timeout(duration, rx).await??)
}

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
