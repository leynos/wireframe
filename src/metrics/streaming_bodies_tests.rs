//! Tests for the bounded streaming-body labels and the occupancy totals.
//!
//! Split from `streaming_bodies.rs` so the module that defines the metrics
//! stays inside the repository's module-length limit. The tests reach the
//! private tally and recorder through `super`, which is why they remain a child
//! of the module under test rather than a separate integration target.

use std::sync::{Arc, Mutex, OnceLock};

use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};

use super::{Occupancy, STREAMING_BODY_OUTCOMES, StreamingBodyOutcome, inc_streaming_body_outcome};

type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

fn sample_matches_outcome(sample: &MetricSample, outcome: StreamingBodyOutcome) -> bool {
    let (key, _, _, value) = sample;
    if key.key().name() != STREAMING_BODY_OUTCOMES || key.key().labels().count() != 1 {
        return false;
    }
    let has_outcome = key
        .key()
        .labels()
        .any(|label| label.key() == "outcome" && label.value() == outcome.as_str());
    has_outcome && matches!(value, DebugValue::Counter(count) if *count == 1)
}

#[test]
fn streaming_body_outcomes_use_a_fixed_label() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let outcomes = [
        StreamingBodyOutcome::Delivered,
        StreamingBodyOutcome::Terminal,
        StreamingBodyOutcome::Abandoned,
        StreamingBodyOutcome::AdmissionTimeout,
        StreamingBodyOutcome::DeliveryTimeout,
        StreamingBodyOutcome::NoHandler,
    ];

    ::metrics::with_local_recorder(&recorder, || {
        for outcome in outcomes {
            inc_streaming_body_outcome(outcome);
        }
    });

    let metric_samples = snapshotter.snapshot().into_vec();
    assert_eq!(metric_samples.len(), outcomes.len());
    for outcome in outcomes {
        assert!(
            metric_samples
                .iter()
                .any(|sample| sample_matches_outcome(sample, outcome)),
            "missing sample for outcome {outcome:?}"
        );
    }
}

/// The totals one tally published, in publication order.
///
/// Each test owns its own sink rather than sharing a process-wide one: the
/// tests run concurrently, and a shared store would let one test drain the
/// totals the other is still asserting on.
#[derive(Clone, Default)]
struct Publications(Arc<Mutex<Vec<usize>>>);

impl Publications {
    /// Return a publisher that records into this sink.
    fn recorder(&self) -> impl Fn(usize) + Sync {
        let published = Arc::clone(&self.0);
        move |total| {
            published
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(total);
        }
    }

    /// Take the totals published so far.
    fn take(&self) -> Vec<usize> {
        let mut published = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *published)
    }
}

/// The tally aggregates additions across connections and releases exactly
/// what was added, so concurrent connections share one total instead of
/// overwriting one another's occupancy.
#[test]
fn occupancy_totals_aggregate_and_release() {
    let publications = Publications::default();
    let bytes = Occupancy::new(publications.recorder());
    assert_eq!(bytes.add(3), 3);
    assert_eq!(bytes.add(4), 7);
    assert_eq!(bytes.release(3), 4);
    assert_eq!(bytes.release(4), 0);

    let parked = Occupancy::new(publications.recorder());
    assert_eq!(parked.add(1), 1);
    assert_eq!(parked.add(1), 2);
    assert_eq!(parked.release(2), 0);
    // A release that races teardown saturates instead of wrapping.
    assert_eq!(parked.release(1), 0);
    assert_eq!(parked.get(), 0);
    assert_eq!(
        publications.take(),
        vec![3, 7, 4, 0, 1, 2, 0, 0],
        "every change publishes the total it computed"
    );
}

/// Take and release one reservation a hundred times.
///
/// Kept out of the spawned closure so the concurrency test reads as one
/// action per thread rather than as nested loops.
fn churn(occupancy: &Occupancy<impl Fn(usize) + Sync>) {
    for _ in 0..100 {
        occupancy.add(1);
        occupancy.release(1);
    }
}

/// A tally publishes the total it computed, in the order it computed it.
///
/// Computing a total outside the lock and publishing it afterwards would
/// let two concurrent releases publish in the opposite order, leaving the
/// gauge above the real total once every reservation had gone. The last
/// publication is therefore the emptied total, whichever thread released
/// last.
#[test]
fn concurrent_updates_publish_their_own_total() {
    let publications = Publications::default();
    let parked = Occupancy::new(publications.recorder());
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| churn(&parked));
        }
    });

    assert_eq!(parked.get(), 0, "every reservation was released");
    assert_eq!(
        publications.take().last().copied(),
        Some(0),
        "the last publication reports an empty total"
    );
}

/// The gauge is published while the update still holds the tally's lock.
///
/// Publishing after the guard is dropped would let a later update install
/// its total first and this one overwrite it, and `Gauge::set` assigns, so
/// the gauge would stay wrong. The probe therefore asks the tally whether
/// its own lock is held from inside the publisher, which a non-reentrant
/// mutex only answers `true` for the update that called it.
#[test]
fn the_total_is_published_while_the_tally_is_locked() {
    type Publisher = Box<dyn Fn(usize) + Send + Sync>;

    let tally: Arc<OnceLock<Occupancy<Publisher>>> = Arc::new(OnceLock::new());
    let probe = Arc::clone(&tally);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let publish: Publisher = Box::new(move |total| {
        // The tally is initialized with this publisher, so it is present
        // whenever the tally can call it.
        let locked = probe.get().is_some_and(Occupancy::is_locked);
        sink.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((total, locked));
    });

    let occupancy = tally.get_or_init(|| Occupancy::new(publish));
    occupancy.add(2);
    occupancy.release(2);

    let published = observed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(published.len(), 2, "each change publishes once");
    assert!(
        published.iter().all(|(_, locked)| *locked),
        "a total must be published while the tally is still locked: {published:?}"
    );
    assert_eq!(
        published.last(),
        Some(&(0, true)),
        "the emptied total is the last value published"
    );
}
