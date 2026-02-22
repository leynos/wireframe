//! Unit tests for interleaved push fairness and rate-limit parity.
//!
//! These tests validate roadmap item 10.3.2 by proving the client-side
//! streaming consumer observes the same queue fairness and rate-limit rules as
//! the server connection actor.

use std::time::Duration;

use futures::{FutureExt, StreamExt};
use rstest::rstest;
use tokio::{sync::mpsc, time};
use tokio_util::sync::CancellationToken;

use super::streaming_infra::{
    CorrelationId,
    MessageId,
    Payload,
    TestStreamEnvelope,
    correlation_id,
    setup_streaming_test,
};
use crate::{
    connection::{ConnectionActor, ConnectionChannels, FairnessConfig},
    hooks::{ConnectionContext, ProtocolHooks},
    push::PushQueues,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn stream_terminator() -> TestStreamEnvelope {
    TestStreamEnvelope {
        id: 0,
        correlation_id: None,
        payload: Vec::new(),
    }
}

fn hooks_with_stream_end() -> ProtocolHooks<TestStreamEnvelope, ()> {
    ProtocolHooks {
        stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| {
            Some(stream_terminator())
        })),
        ..ProtocolHooks::default()
    }
}

fn setup_fairness_actor(
    queues: PushQueues<TestStreamEnvelope>,
    handle: crate::push::PushHandle<TestStreamEnvelope>,
) -> ConnectionActor<TestStreamEnvelope, ()> {
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        CancellationToken::new(),
        hooks_with_stream_end(),
    );
    actor.set_fairness(FairnessConfig {
        max_high_before_low: 1,
        time_slice: None,
    });
    actor
}

async fn setup_multi_packet_channel(
    correlation: CorrelationId,
) -> TestResult<mpsc::Receiver<TestStreamEnvelope>> {
    let (tx, rx) = mpsc::channel(4);
    tx.send(TestStreamEnvelope::data(
        MessageId::new(10),
        correlation,
        Payload::new(vec![10]),
    ))
    .await?;
    tx.send(TestStreamEnvelope::data(
        MessageId::new(11),
        correlation,
        Payload::new(vec![11]),
    ))
    .await?;
    drop(tx);
    Ok(rx)
}

async fn collect_interleaved_fairness_frames(
    correlation: CorrelationId,
) -> TestResult<Vec<TestStreamEnvelope>> {
    let (queues, handle) = PushQueues::<TestStreamEnvelope>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()?;
    let push_handle = handle.clone();

    let mut actor = setup_fairness_actor(queues, handle);

    // High-priority burst with low-priority frames queued to prove fairness.
    push_handle
        .push_high_priority(TestStreamEnvelope::data(
            MessageId::new(1),
            correlation,
            Payload::new(vec![1]),
        ))
        .await?;
    push_handle
        .push_high_priority(TestStreamEnvelope::data(
            MessageId::new(3),
            correlation,
            Payload::new(vec![3]),
        ))
        .await?;
    push_handle
        .push_low_priority(TestStreamEnvelope::data(
            MessageId::new(2),
            correlation,
            Payload::new(vec![2]),
        ))
        .await?;
    push_handle
        .push_low_priority(TestStreamEnvelope::data(
            MessageId::new(4),
            correlation,
            Payload::new(vec![4]),
        ))
        .await?;

    let rx = setup_multi_packet_channel(correlation).await?;

    actor
        .set_multi_packet_with_correlation(Some(rx), Some(correlation.get()))
        .map_err(|e| format!("failed to set multi-packet source: {e}"))?;

    // Allow the actor to observe queue closure once all buffered frames drain.
    drop(push_handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| format!("connection actor run failed: {e:?}"))?;
    Ok(out)
}

async fn collect_rate_limited_frames(
    correlation: CorrelationId,
) -> TestResult<(Vec<TestStreamEnvelope>, bool)> {
    time::pause();

    let (queues, handle) = PushQueues::<TestStreamEnvelope>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(1))
        .build()?;
    let push_handle = handle.clone();

    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        CancellationToken::new(),
        hooks_with_stream_end(),
    );

    push_handle
        .push_high_priority(TestStreamEnvelope::data(
            MessageId::new(1),
            correlation,
            Payload::new(vec![1]),
        ))
        .await?;

    // The limiter budget is shared across priorities, so this low-priority
    // push must remain pending until the next refill window.
    let mut blocked_low = Some(
        push_handle
            .push_low_priority(TestStreamEnvelope::data(
                MessageId::new(2),
                correlation,
                Payload::new(vec![2]),
            ))
            .boxed(),
    );
    tokio::task::yield_now().await;
    let Some(blocked_low_future) = blocked_low.as_mut() else {
        return Err("missing low-priority push future".into());
    };
    let was_blocked = blocked_low_future.as_mut().now_or_never().is_none();
    if was_blocked {
        time::advance(Duration::from_secs(1)).await;
        let Some(fut) = blocked_low.take() else {
            return Err("missing low-priority push future while blocked".into());
        };
        fut.await?;
    }
    let _ = blocked_low.take();
    drop(blocked_low);

    let (tx, rx) = mpsc::channel(1);
    drop(tx);
    actor
        .set_multi_packet_with_correlation(Some(rx), Some(correlation.get()))
        .map_err(|e| format!("failed to set multi-packet source: {e}"))?;

    drop(push_handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| format!("connection actor run failed: {e:?}"))?;

    time::resume();
    Ok((out, was_blocked))
}

fn request_with_correlation(correlation: CorrelationId) -> TestStreamEnvelope {
    TestStreamEnvelope::data(MessageId::new(99), correlation, Payload::new(vec![]))
}

#[rstest]
#[tokio::test]
async fn call_streaming_preserves_interleaved_fairness_order(
    correlation_id: CorrelationId,
) -> TestResult {
    let frames = collect_interleaved_fairness_frames(correlation_id).await?;
    let (mut client, _server) = setup_streaming_test(frames).await?;

    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request_with_correlation(correlation_id))
        .await?;

    let mut payloads = Vec::new();
    while let Some(next) = stream.next().await {
        let frame = next?;
        payloads.push(frame.payload);
    }

    assert_eq!(
        payloads,
        vec![vec![1], vec![2], vec![3], vec![4], vec![10], vec![11]],
        "interleaved high/low fairness ordering regressed",
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn call_streaming_preserves_shared_rate_limit_across_priorities(
    correlation_id: CorrelationId,
) -> TestResult {
    let (frames, was_blocked) = collect_rate_limited_frames(correlation_id).await?;
    assert!(
        was_blocked,
        "expected second push to block under shared cross-priority rate limiting",
    );

    let (mut client, _server) = setup_streaming_test(frames).await?;
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request_with_correlation(correlation_id))
        .await?;

    let mut payloads = Vec::new();
    while let Some(next) = stream.next().await {
        let frame = next?;
        payloads.push(frame.payload);
    }

    assert_eq!(
        payloads,
        vec![vec![1], vec![2]],
        "shared rate limiting should not allow low-priority bypass",
    );
    Ok(())
}
