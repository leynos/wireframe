//! Metrics for streaming request-body delivery.
//!
//! A streaming body can end in several ways that look alike from the outside:
//! the handler may receive every chunk, receive a terminal failure, or lose the
//! exchange to an abandoned body, a bounded admission wait, or a missing
//! handler. The outcome counter separates those cases with a fixed label
//! vocabulary, so an operator can tell a stalled body from a rejected one
//! without reading connection logs.
//!
//! The two occupancy gauges report the work a connection is holding for its
//! bodies: bytes queued for handlers, and completed messages parked behind the
//! one live body. Both are process-wide totals rather than per-connection
//! values, so concurrent connections add to the same series instead of
//! overwriting one another.

use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "metrics")]
use metrics::{counter, gauge};

/// Name of the counter tracking streaming request-body outcomes.
///
/// The `outcome` label uses a fixed vocabulary, so application data cannot
/// create new series.
///
/// ```plaintext
/// # HELP wireframe_streaming_body_outcomes_total Streaming request-body outcomes.
/// # TYPE wireframe_streaming_body_outcomes_total counter
/// wireframe_streaming_body_outcomes_total{outcome="delivered"} 12
/// ```
pub const STREAMING_BODY_OUTCOMES: &str = "wireframe_streaming_body_outcomes_total";

/// Name of the gauge tracking body bytes queued for streaming handlers.
///
/// The value is the total across every connection: a chunk leaves the gauge
/// when its consumer takes it, when the channel drops it, and when a
/// connection ends with chunks still queued.
pub const STREAMING_BODY_BYTES_QUEUED: &str = "wireframe_streaming_body_bytes_queued";

/// Name of the gauge tracking messages parked behind live streaming bodies.
///
/// The value is the total across every connection, and it falls back to zero
/// only when the last parked message is dispatched or its connection drops.
pub const STREAMING_BODIES_PARKED: &str = "wireframe_streaming_bodies_parked";

/// Bounded outcome of one streaming request body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamingBodyOutcome {
    /// The final chunk, or a pre-filled body, reached its handler.
    Delivered,
    /// The handler was told its body failed.
    Terminal,
    /// The body was abandoned before it completed.
    Abandoned,
    /// A chunk was still waiting for byte admission when a bounded wait
    /// expired. Each expired wait is recorded, so one body that waits across
    /// several reads contributes several observations.
    AdmissionTimeout,
    /// A terminal failure could not be delivered within the bounded wait.
    DeliveryTimeout,
    /// A frame arrived for a streaming route with no registered handler.
    NoHandler,
}

impl StreamingBodyOutcome {
    /// Return the stable label for this outcome.
    ///
    /// The same vocabulary serves the metric label and the `error_category`
    /// field of the matching diagnostic event, so the two can be correlated
    /// without a second mapping.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Terminal => "terminal",
            Self::Abandoned => "abandoned",
            Self::AdmissionTimeout => "admission_timeout",
            Self::DeliveryTimeout => "delivery_timeout",
            Self::NoHandler => "no_handler",
        }
    }

    /// Record one body outcome.
    pub(crate) fn record(self) { inc_streaming_body_outcome(self); }
}

/// Record one streaming request-body outcome.
#[cfg(feature = "metrics")]
pub(crate) fn inc_streaming_body_outcome(outcome: StreamingBodyOutcome) {
    counter!(STREAMING_BODY_OUTCOMES, "outcome" => outcome.as_str()).increment(1);
}

/// Record one streaming request-body outcome.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_streaming_body_outcome(_outcome: StreamingBodyOutcome) {}

/// A tally that one process-wide total is kept in.
///
/// Kept as a small type rather than a bare static so the arithmetic can be
/// exercised on a local instance: the statics below are shared with every
/// concurrently running test, so asserting an absolute value on them would be
/// racy.
struct Occupancy {
    /// Current total.
    total: AtomicUsize,
}

impl Occupancy {
    /// Create an empty tally.
    const fn new() -> Self {
        Self {
            total: AtomicUsize::new(0),
        }
    }

    /// Add `count` and return the new total.
    fn add(&self, count: usize) -> usize {
        self.total
            .fetch_add(count, Ordering::SeqCst)
            .saturating_add(count)
    }

    /// Remove `count` and return the new total.
    ///
    /// The exchange is conditional rather than a bare subtraction, because a
    /// release that outruns its additions must floor at zero: an unconditional
    /// `fetch_sub` would wrap the stored value to `usize::MAX` and report an
    /// absurd occupancy for every later reader.
    fn release(&self, count: usize) -> usize {
        let mut current = self.total.load(Ordering::SeqCst);
        loop {
            let next = current.saturating_sub(count);
            match self
                .total
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return next,
                Err(observed) => current = observed,
            }
        }
    }

    /// Return the current total.
    #[cfg(test)]
    fn get(&self) -> usize { self.total.load(Ordering::SeqCst) }
}

/// Bytes currently queued for streaming handlers, across all connections.
static QUEUED_BODY_BYTES: Occupancy = Occupancy::new();

/// Messages currently parked behind live bodies, across all connections.
static PARKED_BODIES: Occupancy = Occupancy::new();

/// Add `len` bytes to the queued-body total.
///
/// Called where a chunk reserves its bytes, so the total already includes a
/// chunk waiting for channel capacity rather than only queued ones.
pub(crate) fn add_queued_body_bytes(len: usize) {
    set_queued_body_bytes(QUEUED_BODY_BYTES.add(len));
}

/// Remove `len` bytes from the queued-body total.
///
/// Called where a chunk's guard is dropped, which covers a consumer taking the
/// chunk, the channel dropping it, and a connection ending with it queued.
pub(crate) fn release_queued_body_bytes(len: usize) {
    set_queued_body_bytes(QUEUED_BODY_BYTES.release(len));
}

/// Add one message to the parked-body total.
pub(crate) fn add_parked_body() { set_parked_bodies(PARKED_BODIES.add(1)); }

/// Remove one message from the parked-body total.
pub(crate) fn release_parked_body() { release_parked_bodies(1); }

/// Remove `count` messages from the parked-body total.
///
/// A connection that ends with messages still parked releases them in one
/// step, so the gauge cannot keep counting work nobody will dispatch.
pub(crate) fn release_parked_bodies(count: usize) {
    set_parked_bodies(PARKED_BODIES.release(count));
}

/// Publish the queued-body total.
///
/// A gauge is an `f64` series, so the count is widened from `usize`. Byte
/// counts beyond `2^52` lose precision, which is accepted here because the
/// gauge reports occupancy at a scale where one part in four quadrillion is
/// not a decision anyone makes.
#[cfg(feature = "metrics")]
#[expect(
    clippy::cast_precision_loss,
    reason = "a gauge series is f64; occupancy beyond 2^52 bytes is not measured"
)]
fn set_queued_body_bytes(total: usize) { gauge!(STREAMING_BODY_BYTES_QUEUED).set(total as f64); }

/// Publish the queued-body total.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
const fn set_queued_body_bytes(_total: usize) {}

/// Publish the parked-body total.
#[cfg(feature = "metrics")]
#[expect(
    clippy::cast_precision_loss,
    reason = "a gauge series is f64; more than 2^52 parked messages cannot exist"
)]
fn set_parked_bodies(total: usize) { gauge!(STREAMING_BODIES_PARKED).set(total as f64); }

/// Publish the parked-body total.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
const fn set_parked_bodies(_total: usize) {}

#[cfg(all(test, feature = "metrics", not(loom)))]
mod tests {
    //! Tests for the bounded streaming-body labels and the occupancy totals.

    use metrics::{SharedString, Unit};
    use metrics_util::{
        CompositeKey,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{
        Occupancy,
        STREAMING_BODY_OUTCOMES,
        StreamingBodyOutcome,
        inc_streaming_body_outcome,
    };

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

    /// The tally aggregates additions across connections and releases exactly
    /// what was added, so concurrent connections share one total instead of
    /// overwriting one another's occupancy.
    #[test]
    fn occupancy_totals_aggregate_and_release() {
        let bytes = Occupancy::new();
        assert_eq!(bytes.add(3), 3);
        assert_eq!(bytes.add(4), 7);
        assert_eq!(bytes.release(3), 4);
        assert_eq!(bytes.release(4), 0);

        let parked = Occupancy::new();
        assert_eq!(parked.add(1), 1);
        assert_eq!(parked.add(1), 2);
        assert_eq!(parked.release(2), 0);
        // A release that races teardown saturates instead of wrapping.
        assert_eq!(parked.release(1), 0);
        assert_eq!(parked.get(), 0);
    }
}
