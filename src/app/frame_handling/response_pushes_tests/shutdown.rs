//! A server stop that lands between push batches must end the run.
//!
//! A writable transport never makes the writes themselves fail, so only the
//! loop's own observation of the stop can end a handler that keeps the run
//! alive with its own pushes. These regressions drive that observation,
//! including the case where another batch is already queued and ready when the
//! stop arrives.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use futures::Sink;
use tokio_util::sync::CancellationToken;

use super::{
    super::response_pushes::run_handler_with_pushes,
    BlockedRigFixture,
    HandlerRig,
    blocked_rig,
    queued_push,
};
use crate::test_helpers::frame_codec::TestFrame;

/// Sink that records every frame and requests shutdown after the first one.
///
/// A writable transport models the reported case: shutdown arrives while the
/// writes themselves always succeed, so only the loop's own shutdown
/// observation can end the run.
struct StopAfterFirstWrite {
    /// Token cancelled once the first frame has been accepted, when set.
    shutdown: Option<CancellationToken>,
    /// Frames the transport accepted, in write order.
    written: Vec<TestFrame>,
}

impl Sink<TestFrame> for StopAfterFirstWrite {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: TestFrame) -> Result<(), Self::Error> {
        self.written.push(item);
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// A handler kept alive by its own pushes must still observe shutdown.
///
/// The sink stays writable, so every push write completes before the write's
/// own shutdown arm could run. Without observing shutdown between batches the
/// run would never end, and the connection could never finish draining.
#[tokio::test]
async fn shutdown_between_push_batches_abandons_the_handler() {
    let shutdown = CancellationToken::new();
    let sink = StopAfterFirstWrite {
        shutdown: Some(shutdown.clone()),
        written: Vec::new(),
    };
    let mut rig = HandlerRig::new(shutdown, sink).expect("actor queues build");
    // The handler pushes once and then never finishes, so only a shutdown that
    // is observed between batches can end the run.
    let call = {
        let producer = rig.handle.clone();
        async move {
            producer
                .push_high_priority(queued_push())
                .await
                .expect("the lane has capacity");
            std::future::pending::<()>().await;
        }
    };

    let response = {
        let mut ctx = rig.context();
        let running = run_handler_with_pushes(&mut ctx, call);
        tokio::time::timeout(Duration::from_secs(5), running)
            .await
            .expect("shutdown between batches must end the run")
            .expect("an abandoned run is not an error")
    };

    assert_eq!(
        response, None,
        "an abandoned handler must not deliver a response"
    );
    assert_eq!(
        rig.sink.written.len(),
        1,
        "the frame written before the stop reached the transport"
    );
    assert_eq!(
        rig.handle.push_high_priority(queued_push()).await,
        Err(crate::push::PushError::Closed),
        "the abandoned run must stop accepting pushes"
    );
}

/// The push a second batch queues while the first one is still being written.
fn second_push() -> TestFrame {
    TestFrame {
        tag: 8,
        payload: vec![8],
    }
}

/// A stop wins over a batch that is already queued behind the pending one.
///
/// The first write is held open, so the handler can queue a second batch before
/// the stop arrives. The stop is therefore observed between batches with the
/// second batch already ready: a run that consulted the lanes before the stop
/// token would deliver it too. The stop is requested only after the queued
/// batch is proven present, and the peer then accepts the first write, so the
/// pending write is ready before the run is polled again. A ready write
/// outranks the stop, which is what lets the test reach the between-batch check
/// instead of abandoning the very write it means to have delivered.
#[rstest::rstest]
#[tokio::test]
async fn shutdown_takes_precedence_over_a_queued_push_batch(blocked_rig: BlockedRigFixture) {
    let (mut rig, release) = blocked_rig.expect("the rig's queues build");
    let shutdown = rig.shutdown.clone();
    // The first batch is already queued by the fixture. The handler queues the
    // second one once the actor has taken the first, and never finishes, so
    // only the stop token can end the run.
    let queued = Arc::new(AtomicBool::new(false));
    let call = {
        let producer = rig.handle.clone();
        let record = Arc::clone(&queued);
        async move {
            producer
                .push_high_priority(second_push())
                .await
                .expect("the drained lane has capacity");
            record.store(true, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
    };

    let response = {
        let mut ctx = rig.context();
        let running = run_handler_with_pushes(&mut ctx, call);
        tokio::pin!(running);
        assert!(
            futures::poll!(running.as_mut()).is_pending(),
            "the first write must still be waiting for the peer"
        );
        assert!(
            queued.load(Ordering::SeqCst),
            "the second batch must be queued while the first write waits"
        );
        // Both events are ready before the run is polled again: the peer
        // accepts the first write, and the stop arrives. The write wins that
        // poll, so the stop is met between batches rather than mid-write.
        shutdown.cancel();
        release
            .send(())
            .expect("the handler run still holds the release channel");
        running.await.expect("an abandoned run is not an error")
    };

    assert_eq!(
        response, None,
        "an abandoned handler must not deliver a response"
    );
    assert_eq!(
        rig.sink.written.len(),
        1,
        "the queued batch must be discarded rather than written"
    );
    assert_eq!(
        rig.sink.written.first().map(|frame| frame.tag),
        Some(queued_push().tag),
        "the only frame written must be the first batch's push"
    );
    assert_eq!(
        rig.try_queue_push(),
        Err(crate::push::PushError::Closed),
        "the abandoned run must stop accepting pushes without waiting for room"
    );
}
