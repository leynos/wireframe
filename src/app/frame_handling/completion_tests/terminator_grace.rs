//! Terminator writes bounded by the peer's idle grace.
//!
//! The protocol terminator is the last frame the prepared path writes, so it
//! must be abandonable exactly like a buffered frame. These cases pin that a
//! terminator the unobservable peer never accepts is abandoned once the grace
//! expires, and that the abandoned write suppresses the completion hook.

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::*;

/// A terminator the unobservable peer never accepts is abandoned after the
/// idle grace.
///
/// The peer stops being observed either because it closed its input half or
/// because a frame was buffered for the next request; both cancel the same
/// observation token. A half-close alone does not mean the peer stopped
/// reading, so the write keeps a whole grace to complete. Only a grace that
/// passes without the sink accepting the frame abandons it, and the abandoned
/// terminator must not run the completion hook.
#[rstest::rstest]
#[case::half_close(true)]
#[case::buffered_request(false)]
#[tokio::test(start_paused = true)]
async fn a_terminator_is_abandoned_when_the_peer_never_accepts_it(
    #[case] close_peer_before_write: bool,
) {
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let (mut actor, _handle) = build_actor().expect("the actor should build");
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
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
    let outcome = driving
        .await
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
