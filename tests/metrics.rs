use metrics_util::debugging::DebuggingRecorder;
use tokio_util::sync::CancellationToken;
use wireframe::{connection::ConnectionActor, push::PushQueues};

#[tokio::test]
async fn outbound_frame_metric_increments() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    recorder.install().expect("install");

    let (queues, handle) = PushQueues::<u8>::bounded(1, 1);
    handle.push_high_priority(1).await.unwrap();
    let token = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics
        .iter()
        .any(|(k, ..)| k.key().name() == wireframe::metrics::FRAMES_PROCESSED);
    assert!(found, "frames_processed metric not recorded");
}
