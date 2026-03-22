//! Unit tests for streaming response helper adapters.

use std::io;

use futures::{StreamExt, TryStreamExt};
use rstest::rstest;

use super::streaming_infra::{
    CorrelationId,
    MessageId,
    Payload,
    correlation_id,
    create_test_client,
    setup_streaming_test,
    spawn_malformed_server,
    spawn_mismatch_server,
    spawn_test_server,
};
use crate::{
    client::{ClientError, StreamingResponseExt},
    correlation::CorrelatableFrame,
};

fn build_request(correlation_id: CorrelationId) -> super::streaming_infra::TestStreamEnvelope {
    super::streaming_infra::TestStreamEnvelope::data(
        MessageId::new(99),
        correlation_id,
        Payload::new(vec![]),
    )
}

async fn collect_typed_items<F>(
    cid: CorrelationId,
    frames: Vec<super::streaming_infra::TestStreamEnvelope>,
    mapper: F,
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(super::streaming_infra::TestStreamEnvelope) -> Result<Option<Vec<u8>>, ClientError>
        + Send
        + Unpin
        + 'static,
{
    let (mut client, _server) = setup_streaming_test(frames).await?;
    let items: Vec<Vec<u8>> = client
        .call_streaming(build_request(cid))
        .await?
        .typed_with(mapper)
        .try_collect()
        .await?;
    Ok(items)
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_yields_mapped_items_in_order(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![10]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(2),
            cid,
            Payload::new(vec![20]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(3),
            cid,
            Payload::new(vec![30]),
        ),
        super::streaming_infra::TestStreamEnvelope::terminator(cid),
    ];
    let items = collect_typed_items(cid, frames, |frame| Ok(Some(frame.payload))).await?;

    assert_eq!(items, vec![vec![10], vec![20], vec![30]]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_skips_control_frames(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![1]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(2),
            cid,
            Payload::new(vec![200]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(3),
            cid,
            Payload::new(vec![2]),
        ),
        super::streaming_infra::TestStreamEnvelope::terminator(cid),
    ];
    let items = collect_typed_items(cid, frames, |frame| {
        if frame.payload == vec![200] {
            Ok(None)
        } else {
            Ok(Some(frame.payload))
        }
    })
    .await?;

    assert_eq!(items, vec![vec![1], vec![2]]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_surfaces_mapper_errors(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![1]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(2),
            cid,
            Payload::new(vec![2]),
        ),
        super::streaming_infra::TestStreamEnvelope::terminator(cid),
    ];
    let (mut client, _server) = setup_streaming_test(frames).await?;

    let mut stream = client
        .call_streaming(build_request(cid))
        .await?
        .typed_with(|frame| {
            if frame.payload == vec![2] {
                Err(ClientError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mapper rejected frame",
                )))
            } else {
                Ok(Some(frame.payload))
            }
        });

    assert_eq!(stream.next().await.transpose()?, Some(vec![1]));

    let error = stream
        .next()
        .await
        .expect("mapper error should be yielded")
        .expect_err("second item should be a mapper error");
    assert!(
        matches!(error, ClientError::Wireframe(_)),
        "expected mapper error to surface as the supplied ClientError"
    );

    assert!(stream.next().await.is_none());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_propagates_correlation_mismatch(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let wrong_cid = CorrelationId::new(correlation_id.get() + 999);
    let server = spawn_mismatch_server(wrong_cid).await?;
    let mut client = create_test_client(server.addr).await?;

    let mut request = build_request(correlation_id);
    request.set_correlation_id(Some(correlation_id.get()));

    let error = client
        .call_streaming(request)
        .await?
        .typed_with(|frame| Ok(Some(frame.payload)))
        .next()
        .await
        .expect("stream error should be yielded")
        .expect_err("expected mismatch error");

    match error {
        ClientError::StreamCorrelationMismatch { expected, received } => {
            assert_eq!(expected, Some(correlation_id.get()));
            assert_eq!(received, Some(wrong_cid.get()));
        }
        other => panic!("expected StreamCorrelationMismatch, got {other:?}"),
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_propagates_disconnects(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![10]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(2),
            cid,
            Payload::new(vec![20]),
        ),
    ];
    let server = spawn_test_server(frames, true).await?;
    let mut client = create_test_client(server.addr).await?;

    let mut stream = client
        .call_streaming(build_request(cid))
        .await?
        .typed_with(|frame| Ok(Some(frame.payload)));

    assert_eq!(stream.next().await.transpose()?, Some(vec![10]));
    assert_eq!(stream.next().await.transpose()?, Some(vec![20]));
    assert!(
        matches!(stream.next().await, Some(Err(ClientError::Wireframe(_)))),
        "disconnect should propagate unchanged"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_propagates_decode_failures(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = spawn_malformed_server().await?;
    let mut client = create_test_client(server.addr).await?;

    let error = client
        .call_streaming(build_request(correlation_id))
        .await?
        .typed_with(|frame| Ok(Some(frame.payload)))
        .next()
        .await
        .expect("decode failure should be yielded")
        .expect_err("expected decode failure");

    assert!(
        matches!(error, ClientError::Wireframe(_)),
        "decode failure should propagate unchanged"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_preserves_empty_streams(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frames = vec![super::streaming_infra::TestStreamEnvelope::terminator(
        correlation_id,
    )];
    let (mut client, _server) = setup_streaming_test(frames).await?;

    let next = client
        .call_streaming(build_request(correlation_id))
        .await?
        .typed_with(|frame| Ok(Some(frame.payload)))
        .next()
        .await;

    assert!(next.is_none(), "empty streams should remain empty");
    Ok(())
}
