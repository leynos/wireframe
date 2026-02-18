//! Unit tests for client streaming response APIs.

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::rstest;
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::streaming_infra::{
    CorrelationId,
    MessageId,
    Payload,
    TestServer,
    TestStreamEnvelope,
    correlation_id,
    create_test_client,
    setup_streaming_test,
    spawn_malformed_server,
    spawn_mismatch_server,
    spawn_test_server,
};
use crate::{BincodeSerializer, Serializer, client::ClientError, correlation::CorrelatableFrame};

/// Verify a stream yields exactly one data frame with the expected payload
/// and then terminates cleanly.
async fn verify_single_frame_stream<S>(
    mut stream: S,
    expected_payload: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: StreamExt<Item = Result<TestStreamEnvelope, ClientError>> + Unpin,
{
    let frame = stream.next().await.expect("data frame").expect("Ok");
    assert_eq!(frame.payload, expected_payload);

    let end = stream.next().await;
    assert!(end.is_none(), "stream should terminate");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn response_stream_yields_data_frames_in_order(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![10])),
        TestStreamEnvelope::data(MessageId::new(2), cid, Payload::new(vec![20])),
        TestStreamEnvelope::data(MessageId::new(3), cid, Payload::new(vec![30])),
        TestStreamEnvelope::terminator(cid),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await?;

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming should succeed");

    let mut received = Vec::new();
    while let Some(result) = stream.next().await {
        received.push(result.expect("data frame should be Ok"));
    }

    assert_eq!(received.len(), 3, "should receive exactly 3 data frames");
    assert_eq!(received.first().expect("frame 0").payload, vec![10]);
    assert_eq!(received.get(1).expect("frame 1").payload, vec![20]);
    assert_eq!(received.get(2).expect("frame 2").payload, vec![30]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn response_stream_terminates_on_terminator(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![1])),
        TestStreamEnvelope::terminator(cid),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await?;

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // First item is the data frame.
    let first = stream.next().await;
    assert!(first.is_some(), "should yield one data frame");
    assert!(first.expect("some").is_ok(), "data frame should be Ok");

    // Second poll returns None (terminator consumed).
    let second = stream.next().await;
    assert!(second.is_none(), "stream should terminate after terminator");

    // Subsequent polls also return None.
    let third = stream.next().await;
    assert!(third.is_none(), "stream should remain terminated");

    assert!(stream.is_terminated(), "is_terminated should be true");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn response_stream_validates_correlation_id(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let wrong_cid = CorrelationId::new(correlation_id.get() + 999);
    let server = spawn_mismatch_server(wrong_cid).await?;
    let mut client = create_test_client(server.addr).await?;

    let mut request =
        TestStreamEnvelope::data(MessageId::new(99), correlation_id, Payload::new(vec![]));
    request.set_correlation_id(Some(correlation_id.get()));

    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    let result = stream.next().await;
    match result {
        Some(Err(ClientError::StreamCorrelationMismatch { expected, received })) => {
            assert_eq!(expected, Some(correlation_id.get()));
            assert_eq!(received, Some(wrong_cid.get()));
        }
        other => panic!("expected StreamCorrelationMismatch, got {other:?}"),
    }

    // After a correlation mismatch, the stream should be marked terminated
    // and yield no further items.
    assert!(
        stream.is_terminated(),
        "stream should be terminated after correlation mismatch"
    );

    let next = stream.next().await;
    assert!(
        next.is_none(),
        "no further items should be yielded after correlation mismatch"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn response_stream_handles_empty_stream(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![TestStreamEnvelope::terminator(cid)];

    let (mut client, _server) = setup_streaming_test(frames).await?;

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Stream should immediately return None (only terminator was sent).
    let first = stream.next().await;
    assert!(
        first.is_none(),
        "empty stream should yield None immediately"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn response_stream_handles_connection_close(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![10])),
        TestStreamEnvelope::data(MessageId::new(2), cid, Payload::new(vec![20])),
    ];

    let server = spawn_test_server(frames, true).await?;
    let mut client = create_test_client(server.addr).await?;

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Should receive the two data frames.
    let first = stream.next().await.expect("first frame").expect("Ok");
    assert_eq!(first.payload, vec![10]);
    let second = stream.next().await.expect("second frame").expect("Ok");
    assert_eq!(second.payload, vec![20]);

    // Next poll should return an error (connection closed without terminator).
    let third = stream.next().await;
    assert!(
        matches!(third, Some(Err(ClientError::Wireframe(_)))),
        "should return a transport error on disconnect, got {third:?}"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn response_stream_surfaces_decode_error(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = spawn_malformed_server().await?;
    let mut client = create_test_client(server.addr).await?;

    let request =
        TestStreamEnvelope::data(MessageId::new(99), correlation_id, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // First poll should yield a decode error.
    let first = stream.next().await;
    assert!(
        matches!(first, Some(Err(ClientError::Wireframe(_)))),
        "expected decode error, got {first:?}"
    );

    // After a decode failure the stream should be terminated.
    assert!(
        stream.is_terminated(),
        "stream should be terminated after decode error"
    );
    let second = stream.next().await;
    assert!(
        second.is_none(),
        "no further items should be yielded after decode error"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn call_streaming_sends_request_and_returns_stream(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![77])),
        TestStreamEnvelope::terminator(cid),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await?;

    // Use explicit correlation ID so server response matches.
    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    assert_eq!(stream.correlation_id(), cid.get());
    verify_single_frame_stream(stream, vec![77]).await
}

#[rstest]
#[tokio::test]
async fn call_streaming_auto_generates_correlation_id()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // This server echoes frames with matching CID, so we need a smarter
    // server that captures the request's CID and uses it in the response.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let _server = TestServer::from_handle(
        addr,
        tokio::spawn(async move {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

            // Read request, extract correlation ID.
            if let Some(Ok(req_bytes)) = transport.next().await {
                let Ok((req, _)) = BincodeSerializer.deserialize::<TestStreamEnvelope>(&req_bytes)
                else {
                    return;
                };
                let Some(cid) = req.correlation_id() else {
                    return;
                };

                // Send one data frame + terminator with matching CID.
                let data = TestStreamEnvelope::data(
                    MessageId::new(1),
                    CorrelationId::new(cid),
                    Payload::new(vec![42]),
                );
                let Ok(encoded) = BincodeSerializer.serialize(&data) else {
                    return;
                };
                let _ = transport.send(Bytes::from(encoded)).await;

                let term = TestStreamEnvelope::terminator(CorrelationId::new(cid));
                let Ok(encoded) = BincodeSerializer.serialize(&term) else {
                    return;
                };
                let _ = transport.send(Bytes::from(encoded)).await;
            }
        }),
    );

    let mut client = create_test_client(addr).await?;

    // Send request without explicit correlation ID.
    let request = TestStreamEnvelope {
        id: 99,
        correlation_id: None,
        payload: vec![],
    };

    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Verify the auto-generated correlation ID is positive.
    assert!(stream.correlation_id() > 0, "should auto-generate CID");

    let frame = stream.next().await.expect("data frame").expect("Ok");
    assert_eq!(frame.payload, vec![42]);

    let end = stream.next().await;
    assert!(end.is_none(), "stream should terminate");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn receive_streaming_works_with_pre_sent_request(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![55])),
        TestStreamEnvelope::terminator(cid),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await?;

    // Send the request manually via send_envelope.
    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let sent_cid = client.send_envelope(request).await.expect("send");

    // Use receive_streaming to consume the response.
    let stream = client.receive_streaming::<TestStreamEnvelope>(sent_cid);
    verify_single_frame_stream(stream, vec![55]).await
}
