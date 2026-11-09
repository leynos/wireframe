//! Assertion helpers for the server-streaming end-to-end tests.
//!
//! `panic_in_result_fn` rejects assertion macros inside tests that return a
//! `Result`, so each check lives in a helper returning `()`. The helpers read
//! values the test has already decoded or counted.

use wireframe::{
    app::Envelope,
    connection::active_connection_count,
    correlation::CorrelatableFrame,
};

use super::{Observations, UNIT_ROUTE};
use crate::stream_types::{CorrelationId, StreamTestEnvelope};

/// Assert the streamed frames arrive in order and carry the request
/// correlation.
pub(super) fn assert_streamed_frames(
    frames: &[StreamTestEnvelope],
    expected: &[&[u8]],
    correlation: CorrelationId,
    origin: &str,
) {
    let payloads: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| frame.payload.clone().into_inner())
        .collect();
    assert_eq!(payloads, expected, "{origin} frames arrive in order");
    for frame in frames {
        assert_eq!(
            frame.correlation_id,
            Some(correlation),
            "every {origin} frame inherits the request correlation"
        );
    }
}

/// Assert the recorded frames are `data_count` correlated data frames followed
/// by the protocol terminator.
pub(super) fn assert_recorded_stream(
    recorded: &[Envelope],
    data_count: usize,
    terminator: &Envelope,
    correlation: CorrelationId,
) {
    assert_eq!(
        recorded.len(),
        data_count + 1,
        "the hook records every data frame and the terminator"
    );
    assert_eq!(
        recorded.last(),
        Some(terminator),
        "the stream ends with the protocol terminator"
    );
    assert!(
        recorded.get(..data_count).is_some_and(|data| data
            .iter()
            .all(|frame| frame.correlation_id() == Some(correlation.get()))),
        "the hook observes the stamped correlation ids"
    );
}

/// Assert the active-connection gauge reports `expected`.
pub(super) fn assert_active_connections(expected: u64) {
    assert_eq!(
        active_connection_count(),
        expected,
        "the active connection gauge must report {expected}"
    );
}

/// Assert the application teardown hook ran exactly once.
pub(super) fn assert_teardown_once(observations: &Observations) {
    assert_eq!(
        observations.teardown_count(),
        1,
        "teardown runs exactly once"
    );
}

/// Assert the unit route replied once, stamped with its route and correlation.
pub(super) fn assert_unit_route_reply(
    reply: &StreamTestEnvelope,
    correlation: CorrelationId,
    recorded_len: usize,
    recorded_route: Option<u32>,
) {
    assert_eq!(
        reply.payload.clone().into_inner(),
        vec![0xab],
        "unit route echoes the request"
    );
    assert_eq!(
        reply.correlation_id,
        Some(correlation),
        "the single reply carries the request correlation"
    );
    assert_eq!(recorded_len, 1, "a unit route sends exactly one frame");
    assert_eq!(
        recorded_route,
        Some(UNIT_ROUTE),
        "the unit reply is stamped with the route id"
    );
}
