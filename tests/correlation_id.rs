#![cfg(not(loom))]
//! Tests for `correlation_id` propagation in streaming responses.
use async_stream::try_stream;
use rstest::rstest;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    CorrelatableFrame,
    app::Envelope,
    connection::ConnectionActor,
    push::PushQueues,
    response::FrameStream,
};

#[tokio::test]
async fn stream_frames_carry_request_correlation_id() {
    let cid = 42u64;
    let stream: FrameStream<Envelope> = Box::pin(try_stream! {
        yield Envelope::new(1, Some(cid), vec![1]);
        yield Envelope::new(1, Some(cid), vec![2]);
    });
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();
    let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert!(out.iter().all(|e| e.correlation_id() == Some(cid)));
}

async fn run_multi_packet_channel(
    request_correlation: Option<u64>,
    frame_correlations: &[Option<u64>],
) -> Vec<Envelope> {
    let capacity = frame_correlations.len().max(1);
    let (tx, rx) = mpsc::channel(capacity);
    for (idx, correlation) in frame_correlations.iter().enumerate() {
        let marker = u8::try_from(idx).expect("frame index fits in u8") + 1;
        let payload = vec![marker];
        tx.send(Envelope::new(1, *correlation, payload))
            .await
            .expect("send frame");
    }
    drop(tx);

    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(2)
        .low_capacity(2)
        .unlimited()
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<Envelope, ()> =
        ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_multi_packet_with_correlation(Some(rx), request_correlation);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    out
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
) {
    let frames = run_multi_packet_channel(request, &initial).await;
    let correlations: Vec<Option<u64>> = frames
        .iter()
        .map(CorrelatableFrame::correlation_id)
        .collect();
    assert_eq!(correlations, expected);
}
