//! Observability for the awaited push path's capacity waits.
//!
//! The push API already answers "was the queue full?" for the *non-blocking*
//! path: a `try_push` that finds no room returns `QueueFull`, and the policies
//! built on it drop or dead-letter the frame. The awaited path has no
//! equivalent answer. An awaited `push_high_priority` against a full queue
//! returns nothing at all — it parks until a consumer frees a slot — so a
//! workload that spent its whole run blocked on back-pressure is
//! indistinguishable, from the outside, from one that never queued.
//!
//! That gap matters for any claim of the form "this scenario saw no
//! back-pressure". A start barrier and a draining consumer make waiting
//! *possible* or *unlikely*; neither makes it *absent*, and neither leaves
//! evidence either way. [`PushWaitCounts`] is the recorder that closes the gap.
//! It lives in the handle and is incremented by
//! [`PushHandle::push_with_priority`](super::PushHandle); the public read
//! surface is [`PushHandle::wait_counters`](super::PushHandle), which returns a
//! [`PushWaitCounters`] snapshot.

use std::sync::atomic::{AtomicU64, Ordering};

/// Counters describing what one producer's awaited pushes had to wait for.
///
/// A plain struct of integers rather than a live handle, so a caller can
/// snapshot it, store it, and subtract one snapshot from another without
/// holding the handle alive.
///
/// # Why this is not `reported_queue_full`
///
/// A non-blocking send that fails asks a different question from a blocking one
/// that parks, and the push API's existing `QueueFull` report answers only the
/// first. A `try_push` against a full queue returns `QueueFull` whether or not
/// anyone subsequently waits; an awaited `push_high_priority` against a full
/// queue returns nothing at all and simply parks. The two are not
/// interchangeable: a workload can be full of awaited back-pressure and report
/// `reported_queue_full: false` for every send, because that flag is set only
/// by the non-blocking path.
///
/// So this counts the awaited path directly. `awaited_capacity_waits` moves
/// exactly when an awaited push found no free slot and had to park, which is
/// the condition a "no back-pressure" claim needs to rule out and a
/// "back-pressure" scenario needs to establish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PushWaitCounters {
    /// Awaited pushes that found their queue full and had to park for capacity.
    ///
    /// Zero means every awaited push in the bracket took a free slot
    /// immediately. Non-zero means at least one push blocked because a producer
    /// outran its consumer, which is back-pressure rather than queue ownership.
    pub awaited_capacity_waits: u64,
    /// Awaited pushes that reached the channel send and probed it for capacity.
    ///
    /// The denominator for the waits above: it distinguishes "no push had to
    /// wait" from "nothing was measured". A zero wait count beside a zero probe
    /// count says the bracket observed nothing, not that the path was clear.
    pub capacity_probes: u64,
}

impl PushWaitCounters {
    /// Counters accumulated since an earlier snapshot.
    ///
    /// Saturates rather than wrapping: the counters only ever increase, so a
    /// larger earlier reading means the two snapshots came from different
    /// handles — a caller error this cannot repair, but which it must not turn
    /// into a huge bogus delta.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::push::PushWaitCounters;
    ///
    /// let before = PushWaitCounters {
    ///     awaited_capacity_waits: 3,
    ///     capacity_probes: 10,
    /// };
    /// let after = PushWaitCounters {
    ///     awaited_capacity_waits: 5,
    ///     capacity_probes: 17,
    /// };
    /// assert_eq!(
    ///     after.since(before),
    ///     PushWaitCounters {
    ///         awaited_capacity_waits: 2,
    ///         capacity_probes: 7,
    ///     }
    /// );
    /// ```
    #[must_use]
    pub const fn since(self, before: Self) -> Self {
        Self {
            awaited_capacity_waits: self
                .awaited_capacity_waits
                .saturating_sub(before.awaited_capacity_waits),
            capacity_probes: self.capacity_probes.saturating_sub(before.capacity_probes),
        }
    }
}

/// The live recorder behind [`PushWaitCounters`].
///
/// Separate from the snapshot because a handle outlives any one reading: the
/// atomics are shared by every clone of a [`PushHandle`](super::PushHandle),
/// and callers take snapshots rather than resetting shared state, so a reading
/// cannot be lost or double-counted by another producer.
///
/// The atomics are `std`'s rather than loom-aliased. These are diagnostics for
/// a benchmark, not a shared invariant any model checks, and the loom models
/// that exercise the push path assert on drop counts instead. Keeping them out
/// of loom's registry also keeps every push in a model from adding two atomic
/// accesses to the state space.
#[derive(Debug, Default)]
pub(crate) struct PushWaitCounts {
    /// Awaited pushes that found no free slot and had to park.
    awaited_capacity_waits: AtomicU64,
    /// Awaited pushes that reached the channel send and probed it.
    capacity_probes: AtomicU64,
}

impl PushWaitCounts {
    /// Record that an awaited push probed its channel for capacity.
    ///
    /// Called before the probe rather than after, so an attempt that finds the
    /// channel closed still counts: the denominator tracks attempts to send,
    /// not successful capacity observations.
    pub(crate) fn record_capacity_probe(&self) {
        self.capacity_probes.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that an awaited push found the channel full and had to park.
    pub(crate) fn record_capacity_wait(&self) {
        self.awaited_capacity_waits.fetch_add(1, Ordering::Relaxed);
    }

    /// Read the counters accumulated so far.
    pub(crate) fn snapshot(&self) -> PushWaitCounters {
        PushWaitCounters {
            awaited_capacity_waits: self.awaited_capacity_waits.load(Ordering::Relaxed),
            capacity_probes: self.capacity_probes.load(Ordering::Relaxed),
        }
    }
}
