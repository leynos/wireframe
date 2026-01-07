//! Error handling tests for the wireframe client.
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within builder patterns are inherently nested"
)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::SinkExt;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::helpers::{
    FailingSerializer,
    spawn_listener,
    test_error_hook_on_disconnect,
    test_with_client,
};
use crate::{
    BincodeSerializer,
    client::{ClientError, WireframeClient, WireframeClientBuilder},
    rewind_stream::RewindStream,
};

/// Connects a client and returns both the client and server stream for custom server behaviour.
///
/// This helper is used by tests that need to manipulate the server stream after connection,
/// such as sending invalid data or dropping the connection at specific times.
async fn connect_with_server<F, C>(
    configure_builder: F,
) -> (
    WireframeClient<BincodeSerializer, RewindStream<TcpStream>, C>,
    TcpStream,
)
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder<BincodeSerializer, (), C>,
    C: Send + 'static,
{
    let (addr, accept) = spawn_listener().await;
    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");
    let server_stream = accept.await.expect("join accept task");
    (client, server_stream)
}

/// Connects a client with a custom serializer, keeping the server alive for the test duration.
///
/// Returns the client while holding the server connection open. The server stream is dropped
/// when the returned client goes out of scope.
async fn test_with_serializer<S, F, C>(
    configure_builder: F,
) -> WireframeClient<S, RewindStream<TcpStream>, C>
where
    S: crate::Serializer + Send + Sync + 'static,
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder<S, (), C>,
    C: Send + 'static,
{
    let (addr, accept) = spawn_listener().await;
    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");
    let _server = accept.await.expect("join accept task");
    client
}

#[tokio::test]
async fn error_callback_invoked_on_receive_error() {
    let error_count = test_error_hook_on_disconnect(|builder, count| {
        builder.on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
    })
    .await;

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on receive error"
    );
}

#[tokio::test]
async fn no_error_hook_does_not_panic() {
    // Server is dropped inside test_with_client after connection
    let mut client = test_with_client(|builder| builder).await;

    // Receive should fail but not panic since there's no error hook
    let result: Result<Vec<u8>, ClientError> = client.receive().await;
    assert!(
        result.is_err(),
        "receive should fail after disconnect without panicking"
    );
}

#[tokio::test]
async fn error_callback_invoked_on_deserialize_error() {
    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let (mut client, server_stream) = connect_with_server(|builder| {
        builder.on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
    })
    .await;

    // Send invalid bincode data via the server stream
    let mut framed = Framed::new(server_stream, LengthDelimitedCodec::new());

    // Send bytes that are not valid bincode for Vec<u8>
    // (bincode expects a length prefix for variable-length types)
    framed
        .send(Bytes::from_static(&[0xff, 0xff, 0xff, 0xff]))
        .await
        .expect("send invalid frame");

    // Try to receive - should fail with deserialization error and invoke error hook
    let result: Result<Vec<u8>, ClientError> = client.receive().await;
    assert!(
        matches!(result, Err(ClientError::Deserialize(_))),
        "receive should fail with deserialization error"
    );

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on deserialize error"
    );
}

#[tokio::test]
async fn error_callback_invoked_on_send_io_error() {
    #[derive(bincode::Encode, bincode::BorrowDecode)]
    struct TestMessage(Vec<u8>);

    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let (mut client, server_stream) = connect_with_server(|builder| {
        builder.on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
    })
    .await;

    // Drop the server side to cause a broken pipe on send
    drop(server_stream);

    // Retry sending until we get an I/O error, with exponential backoff.
    // The OS needs time to propagate the RST/FIN, which varies by platform and load.
    let mut delay = Duration::from_millis(5);
    let mut result = Ok(());
    for _ in 0..5 {
        tokio::time::sleep(delay).await;
        result = client.send(&TestMessage(vec![0u8; 1024])).await;
        if matches!(result, Err(ClientError::Io(_))) {
            break;
        }
        delay *= 2;
    }

    assert!(
        matches!(result, Err(ClientError::Io(_))),
        "send should fail with I/O error after disconnect"
    );

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on send I/O error"
    );
}

#[tokio::test]
async fn error_callback_invoked_on_serialize_error() {
    #[derive(bincode::Encode, bincode::BorrowDecode)]
    struct TestMessage(u32);

    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let mut client = test_with_serializer(|builder| {
        builder.serializer(FailingSerializer).on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
    })
    .await;

    // Try to send - should fail with serialization error and invoke error hook
    let result = client.send(&TestMessage(42)).await;
    assert!(
        matches!(result, Err(ClientError::Serialize(_))),
        "send should fail with serialization error"
    );

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on serialization error"
    );
}
