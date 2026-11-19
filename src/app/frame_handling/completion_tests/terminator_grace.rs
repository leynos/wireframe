//! Terminator writes bounded by the peer's idle grace.
//!
//! The protocol terminator is the last frame the prepared path writes, so it
//! must be abandonable exactly like a buffered frame. These cases pin that a
//! terminator the unobservable peer never accepts keeps its whole grace and is
//! abandoned when that grace passes, and that the abandoned write suppresses
//! the completion hook.

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::*;

/// A terminator the unobservable peer never accepts keeps its whole grace.
///
/// The peer stops being observed either because it closed its input half or
/// because a frame was buffered for the next request; both cancel the same
/// observation token. A half-close alone does not mean the peer stopped
/// reading, so the write keeps a whole grace to complete. The test therefore
/// checks both sides of that deadline — still pending just before it, and
/// abandoned promptly once it passes — because checking only the final
/// outcome would also pass an implementation that abandoned the terminator the
/// moment the peer closed its input half, or one that abandoned it only long
/// after the grace had expired.
#[rstest::rstest]
#[case::half_close(true)]
#[case::buffered_request(false)]
#[tokio::test(start_paused = true)]
async fn a_terminator_keeps_its_whole_grace_before_being_abandoned(
    #[case] close_peer_before_write: bool,
) {
    const GRACE: Duration = Duration::from_secs(5);
    // Margin either side of the deadline, so no assertion depends on the timer
    // firing at exactly its interval.
    const MARGIN: Duration = Duration::from_secs(1);
    // The two advances: the first stops one margin short of the deadline, the
    // second crosses it by a millisecond. Written out rather than derived, so
    // the runtime cannot quietly subtract past zero.
    const BEFORE_DEADLINE: Duration = Duration::from_secs(4);
    const PAST_DEADLINE: Duration = Duration::from_millis(1_001);
    assert_eq!(
        BEFORE_DEADLINE + MARGIN,
        GRACE,
        "the advances must bracket the deadline by one margin"
    );

    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let (mut actor, _handle) = build_actor().expect("the actor should build");
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    actor.set_peer_closed_grace(GRACE);
    let mut sink = StalledSink {
        log: Arc::clone(&log),
    };

    // An empty stream writes only the protocol terminator, so a pending drive
    // proves the terminator itself is what stalled.
    let stream: FrameStream<Envelope, ()> = futures::stream::empty().boxed();
    let mut driving = std::pin::pin!(run_forward(
        &mut sink,
        &mut actor,
        Response::Stream(stream),
        &log
    ));
    if close_peer_before_write {
        peer_closed.cancel();
    } else {
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the terminator write must stall while the peer is still observed"
        );
        peer_closed.cancel();
    }
    // Polling once after the cancellation arms the grace; the write itself is
    // stalled, so only that deadline can end it.
    assert!(
        futures::poll!(driving.as_mut()).is_pending(),
        "an unobservable peer must not abandon the terminator immediately"
    );

    tokio::time::advance(BEFORE_DEADLINE).await;
    assert!(
        futures::poll!(driving.as_mut()).is_pending(),
        "the terminator must keep the whole grace before it is abandoned"
    );

    tokio::time::advance(PAST_DEADLINE).await;
    // The deadline is virtual under a paused clock, so a short bound here still
    // fails an implementation that only abandons the write long after the
    // grace expired, without costing real time.
    let outcome = tokio::time::timeout(Duration::from_millis(100), driving)
        .await
        .expect("the terminator must be abandoned once the grace has passed")
        .expect("an abandoned terminator is not a transport error");

    assert_eq!(
        outcome,
        Dispatch::Stop,
        "an abandoned terminator must stop the loop before it serves another frame"
    );
    let observed = events(&log);
    assert!(
        !observed.contains(&"send"),
        "the abandoned terminator must never reach the sink: {observed:?}"
    );
    assert!(
        !observed.contains(&"command_end"),
        "an abandoned terminator must not run the completion hook: {observed:?}"
    );
}
