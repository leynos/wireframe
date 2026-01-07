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
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::helpers::{FailingSerializer, spawn_listener, test_error_hook_on_disconnect};
use crate::client::{ClientError, WireframeClient};

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
    let (addr, accept) = spawn_listener().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    // Drop server to cause disconnect
    let server = accept.await.expect("join accept task");
    drop(server);

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

    let (addr, accept) = spawn_listener().await;

    let mut client = WireframeClient::builder()
        .on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    // Get server stream and send invalid bincode data
    let server_stream = accept.await.expect("join accept task");
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

    let (addr, accept) = spawn_listener().await;

    let mut client = WireframeClient::builder()
        .on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    // Drop the server side to cause a broken pipe on send
    let server = accept.await.expect("join accept task");
    drop(server);

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

    let (addr, accept) = spawn_listener().await;

    let mut client = WireframeClient::builder()
        .serializer(FailingSerializer)
        .on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    // Keep the server alive so the connection doesn't fail for other reasons
    let _server = accept.await.expect("join accept task");

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
