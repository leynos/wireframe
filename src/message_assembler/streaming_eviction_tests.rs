//! Expiry reporting for streaming assemblies.
//!
//! A streaming eviction is not just bookkeeping: a handler is waiting on a body
//! that will never complete, and only the connection loop can fail it. These
//! tests pin that every path which evicts a streaming assembly leaves the
//! identity reachable by the next detailed purge, whichever public purge the
//! caller used in between.

use super::*;

#[test]
fn expiry_reports_streaming_keys_separately() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(8);
    let start = Instant::now();
    admit_first(&mut state, &first_header(8, 4, None, false), b"aaaa", start);

    let early = state.purge_expired_detailed_at(start + Duration::from_secs(30));
    assert!(
        early.streaming().is_empty(),
        "the assembly has not expired yet"
    );
    assert!(state.is_streaming_key(key));

    let late = state.purge_expired_detailed_at(start + Duration::from_secs(61));
    assert_eq!(
        reported_keys(late.streaming()),
        [key],
        "the expiry is reported for the assembly that expired"
    );
    assert!(
        late.buffered().is_empty(),
        "no buffered assembly was evicted"
    );
    assert_eq!(state.streaming_count(), 0);
}

#[test]
fn the_buffered_only_purge_still_reports_a_streaming_eviction_later() {
    let mut state = state_with_limit(1024);
    let key = MessageKey(9);
    let start = Instant::now();
    admit_first(&mut state, &first_header(9, 4, None, false), b"aaaa", start);

    let now = start + TIMEOUT + Duration::from_secs(1);
    let evicted = state.purge_expired_at(now);
    assert!(
        evicted.is_empty(),
        "the buffered-only signature reports no key"
    );
    assert_eq!(state.streaming_count(), 0, "the expired partial is gone");

    let reported = state.purge_expired_detailed_at(now);
    assert_eq!(
        reported_keys(reported.streaming()),
        [key],
        "a later detailed purge must still tell the body consumer"
    );
}

#[test]
fn the_buffered_only_purge_keeps_an_already_deferred_eviction() {
    let mut state = state_with_limit(1024);
    let start = Instant::now();
    let expired = first_header(1, 4, Some(8), false);
    admit_first(&mut state, &expired, b"abcd", start);

    let later = start + TIMEOUT + Duration::from_secs(1);
    let unrelated = first_header(2, 2, Some(4), false);
    admit_first(&mut state, &unrelated, b"xy", later);

    let evicted = state.purge_expired_at(later);
    assert!(evicted.is_empty(), "no buffered assembly had expired");

    let reported = state.purge_expired_detailed_at(later);
    assert_eq!(
        reported_keys(reported.streaming()),
        [MessageKey(1)],
        "the deferred eviction must survive the buffered-only purge"
    );
}

#[test]
fn admission_defers_an_evicted_streaming_key_to_the_next_purge() {
    let mut state = state_with_limit(1024);
    let start = Instant::now();
    let expired = first_header(1, 4, Some(8), false);
    admit_first(&mut state, &expired, b"abcd", start);

    // Admitting an unrelated message after the timeout purges the stalled one.
    let later = start + TIMEOUT + Duration::from_secs(1);
    let unrelated = first_header(2, 2, Some(4), false);
    admit_first(&mut state, &unrelated, b"xy", later);
    assert!(
        !state.is_streaming_key(MessageKey(1)),
        "the stalled assembly should have been evicted during admission"
    );

    let reported = state.purge_expired_detailed_at(later);
    assert_eq!(
        reported_keys(reported.streaming()),
        [MessageKey(1)],
        "the body consumer must still be told its assembly expired"
    );

    let repeated = state.purge_expired_detailed_at(later);
    assert!(
        repeated.streaming().is_empty(),
        "a deferred eviction should be reported once"
    );
}
