//! Tests for the parked-message count the connection state publishes.
//!
//! The queue is pure storage; the accounting belongs to the state that owns it
//! and to the sink that state was built with. These cases build a state whose
//! sink is a local tally, so an exact assertion is possible without touching
//! the process-wide gauge.

use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "metrics")]
use metrics_util::debugging::{DebugValue, DebuggingRecorder};

use super::ParkedMessage;
use crate::{
    app::{
        Envelope,
        inbound_handler::stream_handles::{LoopState, ParkedAccounting},
    },
    request::QueuedBytes,
};

/// Metric name the parked count is published under.
#[cfg(feature = "metrics")]
const PARKED_GAUGE: &str = crate::metrics::STREAMING_BODIES_PARKED;

/// An empty payload is enough to exercise the accounting.
fn empty_envelope() -> ParkedMessage { ParkedMessage::Envelope(Envelope::new(9, None, Vec::new())) }

/// The parked count is balanced across parking, draining and teardown.
///
/// The accounting lives on the connection state that owns the queue: parking
/// adds one, draining one releases one, and dropping the state releases
/// whatever a connection still held, which is the path an aborted task or an
/// unwinding handler also takes. The sink is the test's own, so the assertion
/// is exact and touches no shared gauge.
#[test]
fn the_parked_count_balances_across_parking_draining_and_teardown() {
    static ADDED: AtomicUsize = AtomicUsize::new(0);
    static RELEASED: AtomicUsize = AtomicUsize::new(0);
    fn add() { ADDED.fetch_add(1, Ordering::SeqCst); }
    fn release(count: usize) { RELEASED.fetch_add(count, Ordering::SeqCst); }
    ADDED.store(0, Ordering::SeqCst);
    RELEASED.store(0, Ordering::SeqCst);

    {
        let mut state = LoopState::with_parked_accounting(
            QueuedBytes::new(),
            ParkedAccounting::new(add, release),
        );
        state
            .park(empty_envelope())
            .expect("the first message fits");
        state
            .park(empty_envelope())
            .expect("the second message fits");
        assert_eq!(
            ADDED.load(Ordering::SeqCst),
            2,
            "each park is reported once"
        );
        assert_eq!(
            RELEASED.load(Ordering::SeqCst),
            0,
            "nothing is released before the queue drains"
        );

        let drained = state.pop_parked();
        assert!(drained.is_some(), "the oldest message is drained");
        assert_eq!(
            RELEASED.load(Ordering::SeqCst),
            1,
            "draining one message releases one"
        );
    }

    assert_eq!(
        RELEASED.load(Ordering::SeqCst),
        2,
        "dropping the state releases the message it still held"
    );
    assert_eq!(ADDED.load(Ordering::SeqCst), 2, "no park is reported twice");
}

/// Run `scenario` under a local recorder and return the totals published.
///
/// The recorder is thread-local, so it captures exactly the publications this
/// thread makes: the number of them is deterministic even though the totals
/// are process-wide and race other tests.
#[cfg(feature = "metrics")]
fn published_totals(scenario: impl FnOnce()) -> Vec<f64> {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    ::metrics::with_local_recorder(&recorder, scenario);
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            if key.key().name() != PARKED_GAUGE {
                return None;
            }
            match value {
                DebugValue::Gauge(total) => Some(total.into_inner()),
                _ => None,
            }
        })
        .collect()
}

/// A connection built the production way publishes to the shared gauge.
///
/// `LoopState::new` is the only place production wires the process-wide parked
/// gauge. Replacing `ParkedAccounting::process_wide` with a no-op leaves the
/// local recorder empty and fails here.
#[cfg(feature = "metrics")]
#[test]
fn a_production_state_publishes_the_shared_parked_gauge() {
    let published = published_totals(|| {
        let mut state = LoopState::new(QueuedBytes::new());
        state
            .park(empty_envelope())
            .expect("the first message fits");
    });

    assert!(
        !published.is_empty(),
        "a production state must publish the parked gauge when it parks a message"
    );
}

/// Every ending reports its release through the accounting it was built with.
///
/// A connection ends by returning, by being cancelled while it awaits, or by
/// unwinding; all three drop the loop state. The sink here counts calls and
/// forwards to the process-wide tallies, so this proves both that the drop
/// releases what the connection still held and that releasing reaches the
/// shared gauge. Counting call sites rather than reading the gauge back is what
/// makes the assertion exact: the gauge is a single process-wide series, and a
/// concurrent test's park would move it between two readings.
#[cfg(feature = "metrics")]
#[test]
fn every_ending_releases_through_the_shared_tallies() {
    static ADDED: AtomicUsize = AtomicUsize::new(0);
    static RELEASED: AtomicUsize = AtomicUsize::new(0);
    fn add() {
        ADDED.fetch_add(1, Ordering::SeqCst);
        crate::metrics::add_parked_body();
    }
    fn release(count: usize) {
        RELEASED.fetch_add(count, Ordering::SeqCst);
        crate::metrics::release_parked_bodies(count);
    }
    ADDED.store(0, Ordering::SeqCst);
    RELEASED.store(0, Ordering::SeqCst);

    {
        let mut state = LoopState::with_parked_accounting(
            QueuedBytes::new(),
            ParkedAccounting::new(add, release),
        );
        state
            .park(empty_envelope())
            .expect("the first message fits");
        state
            .park(empty_envelope())
            .expect("the second message fits");
        assert!(state.pop_parked().is_some(), "one message is drained");
        assert_eq!(
            (
                ADDED.load(Ordering::SeqCst),
                RELEASED.load(Ordering::SeqCst)
            ),
            (2, 1),
            "two parks and one drain are reported before the connection ends"
        );
    }

    assert_eq!(
        (
            ADDED.load(Ordering::SeqCst),
            RELEASED.load(Ordering::SeqCst)
        ),
        (2, 2),
        "the drop that ends the connection releases the message it still held"
    );
}
