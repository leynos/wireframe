//! Shared test helpers for client tests.

use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe_testing::{ServerMode, process_frame};

use crate::{
    client::{ClientError, WireframeClient, WireframeClientBuilder},
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
};

/// Type alias for async hooks that return their input after performing side effects.
///
/// Used by [`counting_hook`] to provide a reusable closure type for lifecycle tests.
pub type CountingHookClosure<T> =
    Arc<dyn Fn(T) -> Pin<Box<dyn Future<Output = T> + Send>> + Send + Sync>;

/// Result alias for fallible client test helpers.
///
/// Arranging a fixture can fail, so helpers propagate the failure to the test
/// body rather than panicking inside the arrangement.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Handle to a task accepting a single connection on a loopback listener.
pub type AcceptHandle = tokio::task::JoinHandle<std::io::Result<TcpStream>>;

/// Binds a listener on an ephemeral loopback port and reports its address.
pub async fn bind_loopback() -> TestResult<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}

/// Spawns a TCP listener and returns the address and a handle to accept a connection.
pub async fn spawn_listener() -> TestResult<(SocketAddr, AcceptHandle)> {
    let (listener, addr) = bind_loopback().await?;
    let accept = tokio::spawn(async move { listener.accept().await.map(|(stream, _)| stream) });
    Ok((addr, accept))
}

/// Handle to a background frame server, reporting any accept failure on join.
pub type FrameServerHandle = tokio::task::JoinHandle<std::io::Result<()>>;

/// Whether a framed I/O error is just the peer having gone away.
///
/// A test client that finishes its work and drops its connection produces one
/// of these, so they end the serve loop normally. Every other error is a real
/// fault the test should see, rather than being reported as clean completion.
pub fn is_expected_disconnect(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

/// Reads the next frame, reporting an ordinary disconnect as end of stream.
///
/// Collapses the read into the two outcomes the serve loop acts on: `Some`
/// frame to handle, or `None` to stop. A disconnect the peer was entitled to
/// make ends the stream, while any other I/O error is returned so it surfaces
/// on join.
async fn next_frame<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
) -> std::io::Result<Option<bytes::BytesMut>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match framed.next().await {
        None => Ok(None),
        Some(Ok(bytes)) => Ok(Some(bytes)),
        Some(Err(error)) if is_expected_disconnect(&error) => Ok(None),
        Some(Err(error)) => Err(error),
    }
}

/// Spawns a loopback server driven by a per-frame handler.
///
/// `state` is threaded through every iteration and returned when the loop ends,
/// so a caller that only serves replies passes `()` while one that records
/// traffic passes its accumulator. `on_frame` returns the reply to send, or
/// `None` to stop serving.
///
/// The loop also stops when the peer disconnects. Read and write failures that
/// are not an ordinary disconnect are returned, so they surface on join.
///
/// # Usage
///
/// Call it with the initial state and a handler, then keep the returned join
/// handle. [`spawn_frame_server`] is the simplest caller: it passes `()` and
/// answers each frame through `process_frame`. `spawn_capturing_server` in
/// `super::request_hooks_support` passes a `Vec<Vec<u8>>` accumulator, pushes
/// every frame into it, and echoes the frame back.
///
/// # Outcome
///
/// Once the client drops its connection the loop ends and the task completes,
/// so awaiting the handle yields `Ok(Ok(state))` with whatever the handler
/// accumulated. Awaiting before the peer disconnects blocks, so drop the client
/// first. A server-side I/O failure yields `Ok(Err(error))`, and a panic in the
/// task yields `Err(join_error)`; `handle.await??` propagates both.
pub async fn spawn_serving_task<S, F>(
    mut state: S,
    mut on_frame: F,
) -> TestResult<(SocketAddr, tokio::task::JoinHandle<std::io::Result<S>>)>
where
    S: Send + 'static,
    F: FnMut(&mut S, bytes::BytesMut) -> Option<Bytes> + Send + 'static,
{
    let (listener, addr) = bind_loopback().await?;

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

        loop {
            let Some(bytes) = next_frame(&mut framed).await? else {
                break;
            };
            let Some(response) = on_frame(&mut state, bytes) else {
                break;
            };
            match framed.send(response).await {
                Ok(()) => {}
                Err(error) if is_expected_disconnect(&error) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(state)
    });

    Ok((addr, handle))
}

/// Spawns a loopback server that answers each length-delimited frame per `mode`.
///
/// The loop stops when `process_frame` declines to answer or the peer
/// disconnects.
///
/// # Usage
///
/// Call it with the mode the test needs — `ServerMode::Echo` to reflect each
/// frame, `ServerMode::Mismatch` to reply with a different correlation
/// identifier — and keep the returned handle. `spawn_envelope_echo_server` and
/// `spawn_mismatched_correlation_server` in `super::messaging` are the two
/// call sites.
///
/// # Outcome
///
/// The task serves frames until the client disconnects. Drop the client, then
/// await the handle: `handle.await??` yields `()` on a clean run and surfaces a
/// server-side I/O failure or a task panic otherwise. Awaiting while the client
/// is still connected blocks, because the serve loop has no reason to end.
pub async fn spawn_frame_server(mode: ServerMode) -> TestResult<(SocketAddr, FrameServerHandle)> {
    spawn_serving_task((), move |(), bytes| {
        process_frame(mode, &bytes).map(Bytes::from)
    })
    .await
}

/// Helper function to test that a builder option is correctly applied to the TCP socket.
pub async fn assert_builder_option<F, A>(configure_builder: F, assert_option: A) -> TestResult
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder,
    A: FnOnce(&WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<TcpStream>>),
{
    let (addr, accept) = spawn_listener().await?;

    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await?;

    assert_option(&client);

    let _server_stream = accept.await??;
    Ok(())
}

/// Helper function to test lifecycle hooks with a connected client.
///
/// The server stream is intentionally dropped after connection, simulating a disconnected
/// server. This allows tests to verify client behaviour when the peer disconnects.
pub async fn test_with_client<F, C>(
    configure_builder: F,
) -> TestResult<WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<TcpStream>, C>>
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder<BincodeSerializer, (), C>,
    C: Send + 'static,
{
    let (addr, accept) = spawn_listener().await?;
    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await?;
    // Server stream is dropped here, simulating a disconnected server.
    let _server = accept.await??;
    Ok(client)
}

/// Creates a counter and an incrementing hook for use in lifecycle tests.
///
/// Returns a tuple of the counter and a closure that increments the counter
/// when invoked and returns the provided value.
pub fn counting_hook<T>() -> (Arc<AtomicUsize>, CountingHookClosure<T>)
where
    T: Send + 'static,
{
    let counter = Arc::new(AtomicUsize::new(0));
    let count = counter.clone();

    let increment = move |value: T| {
        let count = count.clone();
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            value
        }) as Pin<Box<dyn Future<Output = T> + Send>>
    };

    (counter, Arc::new(increment))
}

/// Helper function to test error hook invocation on disconnect.
///
/// Spawns a listener, connects a client configured via the provided closure,
/// disconnects the server, attempts to receive, and returns the error count.
pub async fn test_error_hook_on_disconnect<F, C>(
    configure_builder: F,
) -> TestResult<Arc<AtomicUsize>>
where
    F: FnOnce(
        WireframeClientBuilder,
        Arc<AtomicUsize>,
    ) -> WireframeClientBuilder<BincodeSerializer, (), C>,
    C: Send + 'static,
{
    let error_count = Arc::new(AtomicUsize::new(0));
    let (addr, accept) = spawn_listener().await?;

    let mut client = configure_builder(WireframeClient::builder(), error_count.clone())
        .connect(addr)
        .await?;

    // Drop the server side to cause a disconnection
    let server = accept.await??;
    drop(server);

    // Try to receive - should fail and invoke error hook
    let result: Result<Vec<u8>, ClientError> = client.receive().await;
    assert!(result.is_err(), "receive should fail after disconnect");

    Ok(error_count)
}

/// A serializer that always fails to serialize, used for testing error hooks.
pub struct FailingSerializer;

impl MessageCompatibilitySerializer for FailingSerializer {}

impl Serializer for FailingSerializer {
    fn serialize<M>(&self, _value: &M) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        M: crate::message::EncodeWith<Self>,
    {
        Err(Box::new(std::io::Error::other(
            "forced serialization failure",
        )))
    }

    fn deserialize<M>(
        &self,
        _bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: crate::message::DecodeWith<Self>,
    {
        Err(Box::new(std::io::Error::other(
            "forced deserialization failure",
        )))
    }
}

/// Macro to generate socket option tests.
macro_rules! socket_option_test {
    ($name:ident, $configure:expr, $assert:expr $(,)?) => {
        #[tokio::test]
        async fn $name() {
            $crate::client::tests::helpers::assert_builder_option($configure, $assert)
                .await
                .expect("assert builder option");
        }
    };
}

pub(crate) use socket_option_test;
