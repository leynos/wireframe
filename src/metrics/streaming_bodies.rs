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

#[cfg(feature = "metrics")]
use std::sync::{Mutex, MutexGuard};

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
    /// The label names the counter's `outcome` value. The diagnostic events
    /// that report the same case derive their `error_category` from it, so the
    /// counter and the event cannot describe one case two different ways. An
    /// abandoned body is the exception: the counter labels the delivery
    /// attempt `abandoned` while the event names the body `abandoned_body`.
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
/// racy. The publisher is a parameter for the same reason: a test supplies its
/// own sink, so two tests running concurrently cannot drain each other's
/// recorded totals.
///
/// The tally and its gauge are updated under one lock. Publishing a value
/// computed outside the lock would let a stale observation land last: two
/// releases can each compute their total, then publish in the opposite order,
/// and `Gauge::set` assigns rather than advances, so the gauge would stay above
/// the real total for good.
#[cfg(feature = "metrics")]
struct Occupancy<P> {
    /// Current total, guarded so a publish cannot be reordered against it.
    total: Mutex<usize>,
    /// Gauge publisher for this total.
    publish: P,
}

#[cfg(feature = "metrics")]
impl<P> Occupancy<P>
where
    P: Fn(usize),
{
    /// Create an empty tally that publishes through `publish`.
    const fn new(publish: P) -> Self {
        Self {
            total: Mutex::new(0),
            publish,
        }
    }

    /// Add `count`, publish the result, and return the new total.
    fn add(&self, count: usize) -> usize { self.update(|total| total.saturating_add(count)) }

    /// Remove `count`, publish the result, and return the new total.
    ///
    /// The subtraction saturates rather than wrapping, because a release that
    /// outruns its additions must floor at zero: an unconditional subtraction
    /// would wrap the stored value to `usize::MAX` and report an absurd
    /// occupancy for every later reader.
    fn release(&self, count: usize) -> usize { self.update(|total| total.saturating_sub(count)) }

    /// Apply one change and publish it while the tally is still locked.
    fn update(&self, change: impl FnOnce(usize) -> usize) -> usize {
        let mut total = self.lock();
        *total = change(*total);
        let current = *total;
        (self.publish)(current);
        current
    }

    /// Take the lock, recovering the value from a poisoned guard.
    ///
    /// A panic elsewhere must not stop occupancy accounting. The guarded value
    /// is a plain count, so the poisoned state carries no broken invariant.
    fn lock(&self) -> MutexGuard<'_, usize> {
        self.total
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Return the current total.
    #[cfg(test)]
    fn get(&self) -> usize { *self.lock() }

    /// Return whether some thread currently holds this tally's lock.
    ///
    /// `std::sync::Mutex` is not reentrant, so this reports `true` from inside
    /// the publisher only when the update that called it still holds the lock.
    /// That is exactly the ordering the gauge depends on.
    #[cfg(test)]
    fn is_locked(&self) -> bool { self.total.try_lock().is_err() }
}

/// Bytes currently queued for streaming handlers, across all connections.
#[cfg(feature = "metrics")]
static QUEUED_BODY_BYTES: Occupancy<fn(usize)> = Occupancy::new(set_queued_body_bytes);

/// Messages currently parked behind live bodies, across all connections.
#[cfg(feature = "metrics")]
static PARKED_BODIES: Occupancy<fn(usize)> = Occupancy::new(set_parked_bodies);

/// Add `len` bytes to the queued-body total.
///
/// Called where a chunk reserves its bytes, so the total already includes a
/// chunk waiting for channel capacity rather than only queued ones.
#[cfg(feature = "metrics")]
pub(crate) fn add_queued_body_bytes(len: usize) { QUEUED_BODY_BYTES.add(len); }

/// Add `len` bytes to the queued-body total.
///
/// This function is a no-op when the `metrics` feature is disabled, and the
/// tally and gauge it would update are not compiled then, so a disabled build
/// spends nothing per chunk.
#[cfg(not(feature = "metrics"))]
pub(crate) const fn add_queued_body_bytes(_len: usize) {}

/// Remove `len` bytes from the queued-body total.
///
/// Called where a chunk's guard is dropped, which covers a consumer taking the
/// chunk, the channel dropping it, and a connection ending with it queued.
#[cfg(feature = "metrics")]
pub(crate) fn release_queued_body_bytes(len: usize) { QUEUED_BODY_BYTES.release(len); }

/// Remove `len` bytes from the queued-body total.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) const fn release_queued_body_bytes(_len: usize) {}

/// Add one message to the parked-body total.
#[cfg(feature = "metrics")]
pub(crate) fn add_parked_body() { PARKED_BODIES.add(1); }

/// Add one message to the parked-body total.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) const fn add_parked_body() {}

/// Remove `count` messages from the parked-body total.
///
/// A connection that ends with messages still parked releases them in one
/// step, so the gauge cannot keep counting work nobody will dispatch.
#[cfg(feature = "metrics")]
pub(crate) fn release_parked_bodies(count: usize) { PARKED_BODIES.release(count); }

/// Remove `count` messages from the parked-body total.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) const fn release_parked_bodies(_count: usize) {}

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

/// Publish the parked-body total.
#[cfg(feature = "metrics")]
#[expect(
    clippy::cast_precision_loss,
    reason = "a gauge series is f64; more than 2^52 parked messages cannot exist"
)]
fn set_parked_bodies(total: usize) { gauge!(STREAMING_BODIES_PARKED).set(total as f64); }

/// Tests for the outcome labels and the occupancy totals.
#[cfg(all(test, feature = "metrics", not(loom)))]
#[path = "streaming_bodies_tests.rs"]
mod tests;
