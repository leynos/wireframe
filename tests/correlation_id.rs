#![cfg(not(loom))]
//! Tests for `correlation_id` propagation in streaming responses.
use std::io;

use async_stream::try_stream;
use rstest::rstest;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    CorrelatableFrame,
    app::Envelope,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, ProtocolHooks},
    push::PushQueues,
    response::FrameStream,
};

mod common;
use common::TestResult;

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn stream_frames_carry_request_correlation_id() -> TestResult {
    let cid = 42u64;
    let stream: FrameStream<Envelope> = Box::pin(try_stream! {
        yield Envelope::new(1, Some(cid), vec![1]);
        yield Envelope::new(1, Some(cid), vec![2]);
    });
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .build()?;
    let shutdown = CancellationToken::new();
    let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| io::Error::other(format!("actor run failed: {e:?}")))?;
    assert!(
        out.iter().all(|e| e.correlation_id() == Some(cid)),
        "frames lost correlation id"
    );
    Ok(())
}

async fn run_multi_packet_channel(
    request_correlation: Option<u64>,
    frame_correlations: &[Option<u64>],
    hooks: ProtocolHooks<Envelope, ()>,
) -> TestResult<Vec<Envelope>> {
    let capacity = frame_correlations.len().max(1);
    let (tx, rx) = mpsc::channel(capacity);
    for (idx, correlation) in frame_correlations.iter().enumerate() {
        let marker = (idx + 1) as u64;
        // Test payload is defined to use little-endian marker bytes to mirror wire format.
        #[expect(
            clippy::little_endian_bytes,
            reason = "Test payload mirrors little-endian marker encoding required by the protocol."
        )]
        let payload = marker.to_le_bytes().to_vec();
        tx.send(Envelope::new(1, *correlation, payload)).await?;
    }
    drop(tx);

    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(2)
        .low_capacity(2)
        .unlimited()
        .build()?;
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<Envelope, ()> = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        hooks,
    );
    actor.set_multi_packet_with_correlation(Some(rx), request_correlation);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| io::Error::other(format!("actor run failed: {e:?}")))?;
    Ok(out)
}

#[rstest]
#[case::stamps_request(Some(7), vec![None, Some(99)], vec![Some(7), Some(7)])]
#[case::clears_when_absent(None, vec![None, Some(13)], vec![None, None])]
#[case::preserves_matching(Some(17), vec![Some(17), Some(17)], vec![Some(17), Some(17)])]
#[tokio::test]
async fn multi_packet_frames_apply_expected_correlation(
    #[case] request: Option<u64>,
    #[case] initial: Vec<Option<u64>>,
    #[case] expected: Vec<Option<u64>>,
) -> TestResult {
    let frames = run_multi_packet_channel(request, &initial, ProtocolHooks::default()).await?;
    let correlations: Vec<Option<u64>> = frames
        .iter()
        .map(CorrelatableFrame::correlation_id)
        .collect();
    assert_eq!(
        correlations, expected,
        "unexpected correlation ids: {correlations:?}, expected {expected:?}"
    );
    Ok(())
}

#[rstest]
#[case::terminator_stamped(Some(11), Some(11))]
#[case::terminator_cleared(None, None)]
#[tokio::test]
async fn multi_packet_terminator_applies_correlation(
    #[case] request: Option<u64>,
    #[case] expected: Option<u64>,
) -> TestResult {
    let hooks = ProtocolHooks {
        stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| {
            Some(Envelope::new(255, None, vec![]))
        })),
        ..ProtocolHooks::default()
    };

    let frames = run_multi_packet_channel(request, &[], hooks).await?;
    let [terminator] = frames.as_slice() else {
        return Err(io::Error::other("expected exactly one terminator frame").into());
    };
    assert_eq!(
        terminator.correlation_id(),
        expected,
        "unexpected terminator correlation"
    );
    Ok(())
}
