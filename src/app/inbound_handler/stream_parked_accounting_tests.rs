//! Tests for the parked-message count the connection state publishes.
//!
//! The queue is pure storage; the accounting belongs to the state that owns it
//! and to the sink that state was built with. These cases build a state whose
//! sink is a local tally, so an exact assertion is possible without touching
//! the process-wide gauge.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::ParkedMessage;
use crate::{
    app::{
        Envelope,
        inbound_handler::stream_handles::{LoopState, ParkedAccounting},
    },
    request::QueuedBytes,
};

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
