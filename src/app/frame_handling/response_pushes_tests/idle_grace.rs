//! The idle-grace arm of a push write made while a handler runs.
//!
//! A peer that stops being observable while one of its push writes is still
//! pending is bounded by the connection's idle grace. The abandonment that
//! reports must name the cause that actually ended it, so an operator can tell
//! a stalled peer from a server stop.

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{
    super::{ResponseContext, response_pushes::run_handler_with_pushes},
    BlockingSink,
    DeferredHandler,
    MAX_FRAME_LENGTH,
    build_actor,
    queued_push,
};
use crate::{
    app::codec_driver::FramePipeline,
    metrics::PUSH_WRITES_ABANDONED,
    serializer::BincodeSerializer,
    test_helpers::frame_codec::{TestCodec, TestFrame},
};

/// An idle-grace abandonment is counted with the reason that ended it.
///
/// The peer stops being observable while the push write is still pending and
/// the shutdown token is never cancelled, so only the grace can end the run.
/// The response is therefore abandoned, and the counter must name
/// `idle_grace_expired`; removing that `record_abandonment` call leaves the
/// counter unrecorded and fails this test.
#[cfg(feature = "metrics")]
#[tokio::test(start_paused = true)]
async fn an_idle_grace_abandoned_write_is_counted_as_such() {
    let grace = Duration::from_secs(5);
    let (mut actor, handle) = build_actor(CancellationToken::new()).expect("actor queues build");
    actor.set_peer_closed_grace(grace);
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    handle
        .push_high_priority(queued_push())
        .await
        .expect("the lane has capacity");
    let (_release, blocked) = oneshot::channel();
    let mut sink = BlockingSink::new(blocked);
    let codec = TestCodec::new(MAX_FRAME_LENGTH);
    let serializer = BincodeSerializer;
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let mut ctx = ResponseContext {
        serializer: &serializer,
        sink: &mut sink,
        pipeline: &mut pipeline,
        codec: &codec,
        actor: &mut actor,
        protocol: None,
    };
    let call = DeferredHandler::new(Arc::new(AtomicBool::new(false)));

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let response = {
        let _recorder = ::metrics::set_default_local_recorder(&recorder);
        let running = run_handler_with_pushes(&mut ctx, call);
        tokio::pin!(running);
        assert!(
            futures::poll!(running.as_mut()).is_pending(),
            "the write must be pending before observation ends"
        );
        peer_closed.cancel();
        // The deadline is built only once observation ends, so the run must
        // see the cancellation before the grace can elapse.
        assert!(
            futures::poll!(running.as_mut()).is_pending(),
            "the write must stay pending until the grace expires"
        );
        tokio::time::advance(grace + Duration::from_secs(1)).await;
        running.await.expect("an abandoned write is not an error")
    };

    assert_eq!(
        response, None,
        "a response behind an abandoned write must not be returned"
    );
    assert!(
        sink.written.is_empty(),
        "the abandoned write must not reach the transport"
    );

    assert_idle_grace_counter(&snapshotter);
}

/// Assert the counter holds one abandonment labelled `idle_grace_expired`.
///
/// Asserting the whole label set rather than the presence of a reason also
/// fails a counter that records the abandonment under `shutdown` as well, which
/// would misattribute a stalled peer to a server stop.
fn assert_idle_grace_counter(snapshotter: &Snapshotter) {
    let samples = snapshotter.snapshot().into_vec();
    let counted: Vec<_> = samples
        .iter()
        .filter(|(key, ..)| key.key().name() == PUSH_WRITES_ABANDONED)
        .collect();
    assert_eq!(
        counted.len(),
        1,
        "exactly one push-write abandonment must be counted"
    );
    let Some((key, _, _, value)) = counted.first() else {
        panic!("the idle-grace abandonment must be counted");
    };
    assert!(
        matches!(value, DebugValue::Counter(1)),
        "the idle grace must record exactly one abandonment; got {value:?}"
    );
    let reasons: Vec<_> = key
        .key()
        .labels()
        .filter(|label| label.key() == "reason")
        .map(::metrics::Label::value)
        .collect();
    assert_eq!(
        reasons,
        ["idle_grace_expired"],
        "the counter must name the idle grace as its only reason"
    );
}
