//! Runtime coverage for the idle-grace cancellation warning.
//!
//! The grace path records a metric and a warning when it abandons a response.
//! The metric has unit coverage; this module pins the operator-facing log line,
//! which names why observation stopped and which response was cancelled, so a
//! later edit cannot silently drop the diagnostic operators rely on when a
//! connection stalls.

use std::time::Duration;

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use wireframe_testing::logger;

use super::test_actor;
use crate::{
    app::Envelope,
    connection::ResponseOutcome,
    hooks::ProtocolHooks,
    response::{FrameStream, Response, WireframeError},
};

/// Cancelling a stalled response warns with its cause and identity.
///
/// The actor is put into its bounded drain with a peer that never yields a
/// frame, so the grace expires and the cancellation path runs against a real
/// actor rather than a direct call to the helper.
#[tokio::test(start_paused = true)]
async fn an_idle_grace_cancellation_warns_with_the_response_identity() {
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_peer_closed_grace(Duration::from_secs(5));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let stalled: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending::<
        Result<Envelope, WireframeError<()>>,
    >());
    let mut sink = Vec::new();

    let mut log = logger();
    log.clear();
    peer_closed.cancel();
    let outcome = timeout(
        Duration::from_secs(10),
        actor.install_response(Response::Stream(stalled), Some(7), &mut sink),
    )
    .await
    .expect("the idle grace must bound the wait")
    .expect("a cancelled response is not an error");

    assert_eq!(outcome, ResponseOutcome::Cancelled);

    let warnings: Vec<String> = std::iter::from_fn(|| log.pop())
        .map(|record| record.args().to_owned())
        .collect();
    let warning = warnings
        .iter()
        .find(|message| message.contains("could not be observed"))
        .unwrap_or_else(|| panic!("the grace cancellation must warn; captured: {warnings:#?}"));

    for expected in [
        // An untracked actor has no connection identity to report, so the
        // warning states that rather than omitting the field.
        "cause unknown",
        "accepted no frame for 5s",
        "correlation=Some(7)",
        "cancelling",
    ] {
        assert!(
            warning.contains(expected),
            "the cancellation warning must name {expected}; captured: {warning}"
        );
    }
}
