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
use tokio::net::TcpListener;
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

/// Spawn a test server that accepts one connection and reads all frames
/// sent by the client, collecting them into a shared `Vec<Vec<u8>>`.
///
/// Returns the server, the shared frame store, and the bound address.
///
/// # Errors
///
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_receiving_server() -> Result<
    (TestServer, Arc<tokio::sync::Mutex<Vec<Vec<u8>>>>),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let frames: Arc<tokio::sync::Mutex<Vec<Vec<u8>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let frames_clone = frames.clone();

    let handle = tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

        while let Some(Ok(bytes)) = transport.next().await {
            frames_clone.lock().await.push(bytes.to_vec());
        }
    });

    Ok((TestServer::from_handle(addr, handle), frames))
}

/// Spawn a test server that accepts and then immediately drops the
/// connection, simulating a transport failure.
///
/// # Errors
///
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_dropping_server()
-> Result<TestServer, Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        // Drop the connection immediately to trigger a transport error
        // when the client attempts to write.
        drop(tcp);
    });

    Ok(TestServer::from_handle(addr, handle))
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
