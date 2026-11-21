//! Shared support for the prepared-connection hook tests.
//!
//! These helpers drive real prepared connections over TCP: they wait for the
//! connection's setup callback, address the live connection through the session
//! registry, read the context log the recording protocol keeps, and append the
//! recording protocol's marker to a frame. Keeping them in one module leaves
//! the test file focused on the contracts it asserts. Include this module after
//! `recording_protocol` and `server_harness` with
//! `#[path = "common/prepared_support.rs"] mod prepared_support;`.

use std::time::Duration;

use bytes::Bytes;
use tokio::time::{Instant, sleep};
use wireframe::{push::PushHandle, session::SessionRegistry};
use wireframe_testing::TestResult;

use super::{
    recording_protocol::{HookEvent, HookKind, MARKER, RecordingProtocolFixture},
    recording_protocol_contexts::contexts,
    server_harness::EVENT_TIMEOUT,
};

/// Append the recording protocol's mutation to one frame.
pub fn marked(payload: &[u8]) -> Vec<u8> {
    let mut frame = payload.to_vec();
    frame.push(MARKER);
    frame
}

/// Wait until the prepared connection has dispatched `on_connection_setup`.
pub async fn wait_for_setup(fixture: &RecordingProtocolFixture<Bytes>) -> TestResult<()> {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    while fixture.events().is_empty() {
        if Instant::now() >= deadline {
            return Err("the prepared connection did not complete setup in time".into());
        }
        sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

/// Return the push handle of the single live connection.
pub fn only_registered_handle(registry: &SessionRegistry<Bytes>) -> TestResult<PushHandle<Bytes>> {
    let ids = registry.active_ids();
    let [id] = ids.as_slice() else {
        return Err(format!("expected exactly one live connection, got {}", ids.len()).into());
    };
    registry
        .get(id)
        .ok_or_else(|| "the live connection should publish its push handle".into())
}

/// Assert the hook log records setup, both frames, the terminator and the
/// completion hook, and that the two frames are the handler's own.
///
/// The expected frames are passed as bytes the caller serialized independently
/// of the hook log, so a dispatch that duplicates one frame in place of the
/// other fails the comparison.
pub fn assert_two_frame_event_log(
    fixture: &RecordingProtocolFixture<Bytes>,
    expected: [&[u8]; 2],
    terminator: &[u8],
    context: &str,
) {
    let events = fixture.events();
    let [
        HookEvent::Setup,
        HookEvent::BeforeSend(first),
        HookEvent::BeforeSend(second),
        HookEvent::StreamEnd,
        HookEvent::BeforeSend(observed_terminator),
        HookEvent::CommandEnd,
    ] = events.as_slice()
    else {
        panic!(
            "expected setup, two hooked frames, a hooked terminator, then completion for \
             {context}, got {events:?}"
        );
    };
    let [expected_first, expected_second] = expected;
    assert_eq!(
        first.as_ref(),
        expected_first,
        "{context}: the hook observes the handler's first frame"
    );
    assert_eq!(
        second.as_ref(),
        expected_second,
        "{context}: the hook observes the handler's second frame"
    );
    assert_eq!(
        &observed_terminator[..],
        terminator,
        "the terminator reaches the hook as the protocol supplied it"
    );
}

/// Assert every callback on this connection observed the same context state.
///
/// Setup runs before the first frame and is the callback that learns the
/// identity, so it must lead the observations; every later callback must
/// report the same identifier and peer address, and the identifier must be the
/// one the session registry published. These are the observable consequences
/// of the stronger guarantee the runtime makes structurally: setup receives
/// the very context value later installed in the frame pipeline, so state it
/// leaves there reaches every later callback.
pub fn assert_context_continuity(
    fixture: &RecordingProtocolFixture<Bytes>,
    registry: &SessionRegistry<Bytes>,
) {
    let observed = contexts(fixture);
    let Some((setup, later)) = observed.split_first() else {
        panic!("the prepared connection should record at least the setup context");
    };
    assert_eq!(
        setup.hook,
        HookKind::Setup,
        "setup is the first callback to observe a context"
    );
    let Some(connection_id) = setup.connection_id else {
        panic!("setup should observe the published connection identifier: {setup:?}");
    };
    assert!(
        setup.peer_addr.is_some(),
        "a TCP connection should observe its peer address: {setup:?}"
    );
    assert_eq!(
        registry.active_ids(),
        vec![connection_id],
        "setup observes the identifier the registry published"
    );
    for observation in later {
        assert_eq!(
            observation.connection_id,
            Some(connection_id),
            "every later callback observes the setup identifier: {observation:?}"
        );
        assert_eq!(
            observation.peer_addr, setup.peer_addr,
            "every later callback observes the setup peer address: {observation:?}"
        );
    }
}
