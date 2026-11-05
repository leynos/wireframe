//! Unit tests for streaming response helper adapters.

use std::io;

use futures::StreamExt;
use rstest::rstest;

use super::{
    streaming_helpers_support::{build_request, collect_typed_items},
    streaming_infra::{
        CorrelationId,
        MessageId,
        Payload,
        correlation_id,
        create_test_client,
        setup_streaming_test,
        spawn_malformed_server,
        spawn_mismatch_server,
        spawn_test_server,
    },
};
use crate::{
    client::{ClientError, StreamingResponseExt},
    correlation::CorrelatableFrame,
};

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

    assert_eq!(items, vec![vec![10], vec![20], vec![30]], "items in order");
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

    assert_eq!(items, vec![vec![1], vec![2]], "control frames skipped");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_all_control_frames_yields_no_items(
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
    let items = collect_typed_items(cid, frames, |_frame| Ok(None)).await?;

    assert!(items.is_empty(), "all-control stream yields no items");
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

    assert_eq!(
        stream.next().await.transpose()?,
        Some(vec![1]),
        "first item"
    );

    let error = stream
        .next()
        .await
        .expect("mapper error should be yielded")
        .expect_err("second item should be a mapper error");
    assert!(matches!(error, ClientError::Wireframe(_)), "mapper error");

    assert!(stream.next().await.is_none(), "stream ends after error");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_response_stream_surfaces_mapper_errors_after_skipped_frames(
    correlation_id: CorrelationId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cid = correlation_id;
    let frames = vec![
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![200]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(2),
            cid,
            Payload::new(vec![1]),
        ),
        super::streaming_infra::TestStreamEnvelope::data(
            MessageId::new(3),
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
            if frame.payload == vec![200] {
                Ok(None)
            } else if frame.payload == vec![2] {
                Err(ClientError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mapper rejected frame after skip",
                )))
            } else {
                Ok(Some(frame.payload))
            }
        });

    assert_eq!(
        stream.next().await.transpose()?,
        Some(vec![1]),
        "first item"
    );

    let error = stream
        .next()
        .await
        .expect("mapper error should be yielded")
        .expect_err("second item should be a mapper error");
    assert!(matches!(error, ClientError::Wireframe(_)), "mapper error");

    assert!(stream.next().await.is_none(), "stream ends after error");
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
            assert_eq!(expected, Some(correlation_id.get()), "expected cid");
            assert_eq!(received, Some(wrong_cid.get()), "received correlation id");
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

    assert_eq!(
        stream.next().await.transpose()?,
        Some(vec![10]),
        "first payload"
    );
    assert_eq!(
        stream.next().await.transpose()?,
        Some(vec![20]),
        "second payload"
    );
    assert!(
        matches!(stream.next().await, Some(Err(ClientError::Wireframe(_)))),
        "disconnect must propagate"
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

    assert!(matches!(error, ClientError::Wireframe(_)), "decode error");
    Ok(())
}

#[tokio::test]
async fn typed_response_stream_fuses_after_mapper_error() {
    let inner = futures::stream::iter(vec![Ok(1u8), Ok(2u8)]);
    let mut stream = inner.typed_with(|_frame| -> Result<Option<u8>, ClientError> {
        Err(ClientError::from(io::Error::new(
            io::ErrorKind::InvalidData,
            "mapper always fails",
        )))
    });

    let first = stream.next().await;
    assert!(
        matches!(first, Some(Err(ClientError::Wireframe(_)))),
        "mapper error on first poll"
    );

    let second = stream.next().await;
    assert!(second.is_none(), "stream must be fused after error");

    let third = stream.next().await;
    assert!(third.is_none(), "stream remains fused");
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

    assert!(next.is_none(), "empty stream stays empty");
    Ok(())
}
