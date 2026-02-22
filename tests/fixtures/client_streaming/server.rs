//! Test server helper functions for client streaming BDD tests.

use std::time::Duration;

use futures::{FutureExt, SinkExt};
use tokio::{sync::mpsc, time};
use tokio_util::{
    codec::{Framed, LengthDelimitedCodec},
    sync::CancellationToken,
};
use wireframe::{
    connection::{ConnectionActor, ConnectionChannels, FairnessConfig},
    hooks::{ConnectionContext, ProtocolHooks},
    push::PushQueues,
};

use super::{
    TestResult,
    types::{CorrelationId, MessageId, Payload, StreamTestEnvelope},
};

/// Send a single frame with a mismatched correlation ID.
pub(crate) async fn send_mismatch_frame<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: CorrelationId,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let bad = StreamTestEnvelope::data(
        MessageId::new(1),
        CorrelationId::new(cid.get() + 999),
        Payload::new(vec![99]),
    );
    if let Ok(encoded) = bad.serialize_to_bytes() {
        let _ = framed.send(encoded).await;
    }
}

/// Send `count` data frames with payload `[1], [2], ..., [count]`.
pub(crate) async fn send_data_frames<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: CorrelationId,
    count: usize,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    for i in 1..=count {
        let Ok(id) = u32::try_from(i) else { break };
        let Ok(payload_byte) = u8::try_from(i) else {
            break;
        };
        let frame =
            StreamTestEnvelope::data(MessageId::new(id), cid, Payload::new(vec![payload_byte]));
        let Ok(encoded) = frame.serialize_to_bytes() else {
            break;
        };
        if framed.send(encoded).await.is_err() {
            break;
        }
    }
}

/// Send `count` data frames followed by a terminator.
pub(crate) async fn send_data_and_terminator<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: CorrelationId,
    count: usize,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_data_frames(framed, cid, count).await;
    let term = StreamTestEnvelope::terminator(cid);
    if let Ok(encoded) = term.serialize_to_bytes() {
        let _ = framed.send(encoded).await;
    }
}

fn stream_terminator() -> StreamTestEnvelope {
    StreamTestEnvelope {
        id: MessageId::new(0),
        correlation_id: None,
        payload: Payload::new(vec![]),
    }
}

fn hooks_with_stream_end() -> ProtocolHooks<StreamTestEnvelope, ()> {
    ProtocolHooks {
        stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| {
            Some(stream_terminator())
        })),
        ..ProtocolHooks::default()
    }
}

/// Build frames using the connection actor with fairness enabled so
/// low-priority traffic interleaves with high-priority bursts.
pub(crate) async fn build_interleaved_priority_frames(
    cid: CorrelationId,
) -> TestResult<Vec<StreamTestEnvelope>> {
    let (queues, handle) = PushQueues::<StreamTestEnvelope>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()?;
    let push_handle = handle.clone();

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

    push_handle
        .push_high_priority(StreamTestEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![1]),
        ))
        .await?;
    push_handle
        .push_high_priority(StreamTestEnvelope::data(
            MessageId::new(3),
            cid,
            Payload::new(vec![3]),
        ))
        .await?;
    push_handle
        .push_low_priority(StreamTestEnvelope::data(
            MessageId::new(2),
            cid,
            Payload::new(vec![2]),
        ))
        .await?;
    push_handle
        .push_low_priority(StreamTestEnvelope::data(
            MessageId::new(4),
            cid,
            Payload::new(vec![4]),
        ))
        .await?;

    let (tx, rx) = mpsc::channel(4);
    tx.send(StreamTestEnvelope::data(
        MessageId::new(10),
        cid,
        Payload::new(vec![10]),
    ))
    .await?;
    tx.send(StreamTestEnvelope::data(
        MessageId::new(11),
        cid,
        Payload::new(vec![11]),
    ))
    .await?;
    drop(tx);

    actor
        .set_multi_packet_with_correlation(Some(rx), Some(cid.get()))
        .map_err(|e| format!("set_multi_packet_with_correlation failed: {e}"))?;
    drop(push_handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| format!("connection actor run failed: {e:?}"))?;
    Ok(out)
}

/// Build frames with a shared one-token-per-second rate limiter.
///
/// Returns the produced frames and whether the low-priority push was observed
/// as blocked until the refill window.
pub(crate) async fn build_rate_limited_priority_frames(
    cid: CorrelationId,
) -> TestResult<(Vec<StreamTestEnvelope>, bool)> {
    time::pause();

    let (queues, handle) = PushQueues::<StreamTestEnvelope>::builder()
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
        .push_high_priority(StreamTestEnvelope::data(
            MessageId::new(1),
            cid,
            Payload::new(vec![1]),
        ))
        .await?;

    let mut blocked_low = Some(
        push_handle
            .push_low_priority(StreamTestEnvelope::data(
                MessageId::new(2),
                cid,
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
        .set_multi_packet_with_correlation(Some(rx), Some(cid.get()))
        .map_err(|e| format!("set_multi_packet_with_correlation failed: {e}"))?;
    drop(push_handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| format!("connection actor run failed: {e:?}"))?;

    time::resume();
    Ok((out, was_blocked))
}
