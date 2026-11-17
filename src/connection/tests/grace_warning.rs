//! Runtime coverage for the idle-grace cancellation warning.
//!
//! The grace path records a metric and a warning when it abandons a response.
//! The metric has unit coverage; this module pins the operator-facing log line,
//! which names why observation stopped and which response was cancelled, so a
//! later edit cannot silently drop the diagnostic operators rely on when a
//! connection stalls. Both recorded causes are driven through a real actor, so
//! the warning's cause is the one the inbound loop would have recorded rather
//! than a value asserted only in isolation.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use wireframe_testing::logger;

use super::test_actor;
use crate::{
    app::Envelope,
    connection::{ResponseOutcome, UnobservedCause},
    hooks::ProtocolHooks,
    response::{FrameStream, Response, WireframeError},
};

/// Cancelling a stalled response warns with its cause and identity.
///
/// The actor is put into its bounded drain with a peer that never yields a
/// frame, so the grace expires and the cancellation path runs against a real
/// actor rather than a direct call to the helper. The cause cell is filled the
/// way the inbound loop fills it, so the warning text under test is the one an
/// operator sees for a half-close or a buffered request.
#[rstest::rstest]
#[case::half_close(UnobservedCause::HalfClose, "a half-close", 9_031)]
#[case::buffered_request(UnobservedCause::BufferedRequest, "a buffered request", 9_032)]
#[tokio::test(start_paused = true)]
async fn an_idle_grace_cancellation_warns_with_the_recorded_cause(
    #[case] cause: UnobservedCause,
    #[case] expected_cause: &str,
    #[case] correlation: u64,
) {
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_peer_closed_grace(Duration::from_secs(5));
    let peer_closed = CancellationToken::new();
    let recorded_cause = Arc::new(OnceLock::new());
    recorded_cause
        .set(cause)
        .expect("the cause cell starts empty");
    actor.set_peer_observation(peer_closed.clone(), Arc::clone(&recorded_cause));
    let stalled: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending::<
        Result<Envelope, WireframeError<()>>,
    >());
    let mut sink = Vec::new();

    let mut log = logger();
    log.clear();
    peer_closed.cancel();
    let outcome = timeout(
        Duration::from_secs(10),
        actor.install_response(Response::Stream(stalled), Some(correlation), &mut sink),
    )
    .await
    .expect("the idle grace must bound the wait")
    .expect("a cancelled response is not an error");

    assert_eq!(outcome, ResponseOutcome::Cancelled);

    let warnings: Vec<String> = std::iter::from_fn(|| log.pop())
        .map(|record| record.args().to_owned())
        .collect();
    // The `log` crate is process-global, so another test's cancellation can
    // land in this window. Each case uses a correlation id no other test
    // installs, so selecting on it finds this case's own record rather than
    // whichever warning happens to arrive first.
    let correlation_field = format!("correlation=Some({correlation})");
    let warning = warnings
        .iter()
        .find(|message| {
            message.contains("could not be observed") && message.contains(&correlation_field)
        })
        .unwrap_or_else(|| panic!("the grace cancellation must warn; captured: {warnings:#?}"));

    // The bound the grace actually applied is part of the diagnostic: an
    // operator needs the interval that elapsed, not just the fact that one did.
    // The fixture sets five seconds, so the rendered field is `5s`.
    for expected in [
        expected_cause,
        correlation_field.as_str(),
        "cancelling",
        "accepted no frame for 5s",
    ] {
        assert!(
            warning.contains(expected),
            "the cancellation warning must name {expected}; captured: {warning}"
        );
    }
    assert!(
        !warning.contains("cause unknown"),
        "the warning must report the recorded cause rather than an unknown one; captured: \
         {warning}"
    );
}
