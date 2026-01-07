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

use tokio::net::{TcpListener, TcpStream};

use crate::{
    BincodeSerializer,
    client::{ClientError, WireframeClient, WireframeClientBuilder},
};

/// Type alias for async hooks that return their input after performing side effects.
///
/// Used by [`counting_hook`] to provide a reusable closure type for lifecycle tests.
pub type CountingHookClosure<T> =
    Arc<dyn Fn(T) -> Pin<Box<dyn Future<Output = T> + Send>> + Send + Sync>;

/// Spawns a TCP listener and returns the address and a handle to accept a connection.
pub async fn spawn_listener() -> (SocketAddr, tokio::task::JoinHandle<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        stream
    });
    (addr, accept)
}

/// Helper function to test that a builder option is correctly applied to the TCP socket.
pub async fn assert_builder_option<F, A>(configure_builder: F, assert_option: A)
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder,
    A: FnOnce(&WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<TcpStream>>),
{
    let (addr, accept) = spawn_listener().await;

    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");

    assert_option(&client);

    let _server_stream = accept.await.expect("join accept task");
}

/// Helper function to test lifecycle hooks with a connected client.
///
/// The server stream is intentionally dropped after connection, simulating a disconnected
/// server. This allows tests to verify client behaviour when the peer disconnects.
pub async fn test_with_client<F, C>(
    configure_builder: F,
) -> WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<TcpStream>, C>
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder<BincodeSerializer, (), C>,
    C: Send + 'static,
{
    let (addr, accept) = spawn_listener().await;
    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");
    // Server stream is dropped here, simulating a disconnected server.
    let _server = accept.await.expect("join accept task");
    client
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
pub async fn test_error_hook_on_disconnect<F, C>(configure_builder: F) -> Arc<AtomicUsize>
where
    F: FnOnce(
        WireframeClientBuilder,
        Arc<AtomicUsize>,
    ) -> WireframeClientBuilder<BincodeSerializer, (), C>,
    C: Send + 'static,
{
    let error_count = Arc::new(AtomicUsize::new(0));
    let (addr, accept) = spawn_listener().await;

    let mut client = configure_builder(WireframeClient::builder(), error_count.clone())
        .connect(addr)
        .await
        .expect("connect client");

    // Drop the server side to cause a disconnection
    let server = accept.await.expect("join accept task");
    drop(server);

    // Try to receive - should fail and invoke error hook
    let result: Result<Vec<u8>, ClientError> = client.receive().await;
    assert!(result.is_err(), "receive should fail after disconnect");

    error_count
}

/// A serializer that always fails to serialize, used for testing error hooks.
pub struct FailingSerializer;

impl crate::Serializer for FailingSerializer {
    fn serialize<M: crate::message::Message>(
        &self,
        _value: &M,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::new(std::io::Error::other(
            "forced serialization failure",
        )))
    }

    fn deserialize<M: crate::message::Message>(
        &self,
        _bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>> {
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
            $crate::client::tests::helpers::assert_builder_option($configure, $assert).await;
        }
    };
}

pub(crate) use socket_option_test;
