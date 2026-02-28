//! Shared test infrastructure for outbound streaming send tests.
//!
//! Provides a receiving server, an in-memory body source helper, and
//! factory functions for creating connected clients with configurable
//! frame lengths.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures::StreamExt;
use rstest::fixture;
use tokio::{io::AsyncWriteExt, net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::streaming_infra::TestServer;
use crate::client::WireframeClient;

/// Default maximum frame length used in send-streaming tests.
///
/// This must match the default `ClientCodecConfig::max_frame_length` (1024).
pub(super) const DEFAULT_MAX_FRAME: usize = 1024;

/// A small header used across most tests.
#[rustfmt::skip]
#[fixture]
pub(super) fn protocol_header() -> Vec<u8> {
    vec![0xCA, 0xFE, 0xBA, 0xBE]
}

/// A test server that collects all frames sent by the client.
///
/// Frames are returned by awaiting the server task via
/// [`collect_frames`](Self::collect_frames), which provides deterministic
/// synchronisation without fixed sleeps.
pub(super) struct ReceivingServer {
    pub addr: SocketAddr,
    handle: Option<JoinHandle<Vec<Vec<u8>>>>,
}

impl ReceivingServer {
    /// Await the server task and return the collected frames.
    ///
    /// The caller must drop the client beforehand so the server sees EOF
    /// and the task completes.
    ///
    /// # Errors
    ///
    /// Returns an error if the server task panicked or was already
    /// collected.
    pub(super) async fn collect_frames(
        &mut self,
    ) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let handle = self
            .handle
            .take()
            .ok_or("server handle already collected")?;
        handle
            .await
            .map_err(|e| format!("server task panicked: {e}").into())
    }
}

impl Drop for ReceivingServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Spawn a test server that accepts one connection and reads all frames
/// sent by the client, returning them when the connection closes.
///
/// # Errors
///
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_receiving_server()
-> Result<ReceivingServer, Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        let mut collected = Vec::new();
        let Ok((tcp, _)) = listener.accept().await else {
            return collected;
        };
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());
        while let Some(Ok(bytes)) = transport.next().await {
            collected.push(bytes.to_vec());
        }
        collected
    });

    Ok(ReceivingServer {
        addr,
        handle: Some(handle),
    })
}

/// Spawn a test server that accepts and then immediately shuts down the
/// write side, triggering a deterministic transport error on the client.
///
/// Returns the server together with a `Notify` that is signalled once the
/// write-side shutdown has completed, so callers can synchronise
/// deterministically instead of sleeping.
///
/// # Errors
///
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_dropping_server()
-> Result<(TestServer, Arc<tokio::sync::Notify>), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let shutdown_done = Arc::new(tokio::sync::Notify::new());
    let notify = shutdown_done.clone();

    let handle = tokio::spawn(async move {
        let Ok((mut tcp, _)) = listener.accept().await else {
            return;
        };
        // Shut down the write side to trigger a deterministic transport
        // error when the client attempts to write.
        let _ = tcp.shutdown().await;
        notify.notify_one();
    });

    Ok((TestServer::from_handle(addr, handle), shutdown_done))
}

/// Create a connected `WireframeClient` for the given address.
///
/// # Errors
///
/// Returns an error if the connection fails.
pub(super) async fn create_send_client(
    addr: SocketAddr,
) -> Result<
    WireframeClient<
        crate::serializer::BincodeSerializer,
        crate::rewind_stream::RewindStream<tokio::net::TcpStream>,
    >,
    Box<dyn std::error::Error + Send + Sync>,
> {
    create_send_client_with_max_frame(addr, DEFAULT_MAX_FRAME).await
}

/// Create a connected `WireframeClient` with a custom max frame length.
///
/// # Errors
///
/// Returns an error if the connection fails.
pub(super) async fn create_send_client_with_max_frame(
    addr: SocketAddr,
    max_frame_length: usize,
) -> Result<
    WireframeClient<
        crate::serializer::BincodeSerializer,
        crate::rewind_stream::RewindStream<tokio::net::TcpStream>,
    >,
    Box<dyn std::error::Error + Send + Sync>,
> {
    Ok(WireframeClient::builder()
        .max_frame_length(max_frame_length)
        .connect(addr)
        .await?)
}

/// Create a connected client with an error hook that sets an `AtomicBool`.
///
/// # Errors
///
/// Returns an error if the connection fails.
pub(super) async fn create_send_client_with_error_hook(
    addr: SocketAddr,
) -> Result<
    (
        WireframeClient<
            crate::serializer::BincodeSerializer,
            crate::rewind_stream::RewindStream<tokio::net::TcpStream>,
        >,
        Arc<AtomicBool>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let hook_invoked = Arc::new(AtomicBool::new(false));
    let flag = hook_invoked.clone();
    let client = WireframeClient::builder()
        .on_error(move |_err| {
            let flag = flag.clone();
            async move {
                flag.store(true, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await?;
    Ok((client, hook_invoked))
}

/// Generate a body of `n` bytes for testing: cycles through 0–255.
pub(super) fn test_body(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| u8::try_from(i.wrapping_rem(256)).unwrap_or(0))
        .collect()
}

type ByteResult = Result<bytes::Bytes, std::io::Error>;
type BlockingStream = tokio_stream::wrappers::ReceiverStream<ByteResult>;

/// An `AsyncRead` backed by an mpsc channel, together with the sender
/// handle that keeps the channel open.
pub(super) type BlockingReader = (
    tokio_util::io::StreamReader<BlockingStream, bytes::Bytes>,
    tokio::sync::mpsc::Sender<ByteResult>,
);

/// Create an `AsyncRead` that blocks indefinitely.
///
/// Returns the reader and the sender handle. The caller must hold the
/// sender until the test assertion is complete — dropping it causes the
/// reader to see EOF, which would race with the timeout under test.
pub(super) fn blocking_reader() -> BlockingReader {
    let (tx, rx) = tokio::sync::mpsc::channel::<ByteResult>(1);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let reader = tokio_util::io::StreamReader::new(stream);
    (reader, tx)
}
