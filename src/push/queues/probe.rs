//! Loom-only inspection of a push handle's shared state.

use super::{handle::PushHandleInner, sync::Arc};

/// Instrumentation helper exposing internal counters when running under loom.
pub struct PushHandleProbe<F> {
    /// The handle's shared state, read without taking part in any push.
    inner: Arc<PushHandleInner<F>>,
}

impl<F> PushHandleProbe<F> {
    /// Wrap a handle's shared state for inspection.
    pub(super) const fn new(inner: Arc<PushHandleInner<F>>) -> Self { Self { inner } }

    /// Return the number of frames dropped into the DLQ since the last log flush.
    #[must_use]
    pub fn dlq_drop_count(&self) -> usize { self.inner.dead_letter.pending_count() }

    /// Return the total number of drops the throttled diagnostic has reported.
    #[must_use]
    pub fn dlq_reported_count(&self) -> usize { self.inner.dead_letter.reported_count() }
}
