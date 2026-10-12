//! Workloads and measurement for the push handle's send path.
//!
//! The push handle is the producer-facing half of a connection: application
//! code, background workers, and protocol hooks all enqueue frames through it.
//! Two distinct costs live here, and the issue asks for them separately:
//!
//! - the **uncontended** cost of awaiting a send into a queue a consumer is draining, split by
//!   priority because the actor polls the high-priority queue first;
//! - the **back-pressure** cost of that same awaited send when the queue is full and no consumer is
//!   running, which is a wait rather than a failure; and
//! - the **overflow** cost of a non-blocking send against a full queue, which branches on
//!   [`PushPolicy`](wireframe::push::PushPolicy) and never awaits capacity.
//!
//! Rate limiting is disabled in every scenario. The default limiter is a leaky
//! bucket capped at 100 pushes per second, so leaving it enabled would make the
//! measured cost almost entirely `leaky_bucket` bookkeeping and sleeping rather
//! than anything about queue ownership.
//!
//! # Drop policies
//!
//! [`PushScenario::OverflowReturnError`] reports
//! [`PushError::QueueFull`](wireframe::push::PushError::QueueFull).
//! [`PushScenario::OverflowDrop`] and [`PushScenario::OverflowWarnAndDrop`]
//! report success and route the frame to the dead-letter queue instead.
//!
//! The two outcomes are not equally observable. Dropped frames are *routed* to
//! the DLQ, so a receiver on that channel sees each one; the handle's own
//! loss counter is `pub(crate)` and increments only when the DLQ itself
//! is full or closed, so it is neither reachable nor the right number from
//! outside the crate. [`measure_push`] therefore reports
//! [`PushMeasurement::dead_lettered`], drained from the DLQ receiver, which is
//! the black-box observation the issue's method asks for.
//!
//! # Awaited back-pressure, which `reported_queue_full` does not cover
//!
//! The awaited path has two regimes, and they are not distinguished by any
//! count the push API returns. A send into a queue with room completes; a send
//! into a full queue parks until a consumer frees a slot; both return `Ok(())`
//! with no other trace. [`PushScenario::BackpressureHigh`] exists because the
//! uncontended scenarios *intend* never to park and had no way to show it.
//!
//! `reported_queue_full` cannot make that distinction: it is set only by the
//! non-blocking path, so it is `false` for every awaited scenario however full
//! the queue was. [`PushMeasurement::awaited_capacity_waits`] is the observable
//! that can, and both awaited scenarios assert it from opposite sides — zero for
//! [`PushScenario::UncontendedHigh`], non-zero for
//! [`PushScenario::BackpressureHigh`]. The uncontended regime also requires
//! [`PushMeasurement::capacity_probes`] to be non-zero, so "no waits" cannot be
//! satisfied by having measured nothing at all.
//!
//! A start barrier and a draining consumer make waiting *possible*; only the
//! counter makes it *absent*. A task that wins the scheduler can complete
//! several immediately-ready sends before another is polled, so the shape of a
//! concurrent burst is not evidence about any individual send.
//!
//! # Layout
//!
//! `scenario` describes a workload and what it must report; `measurement`
//! drives it. The three send regimes live in their own submodules there — the
//! two awaited ones together, since they share a call path, and the
//! non-blocking one separately — because a shared body would obscure which
//! branch a change affected.

mod measurement;
mod scenario;

#[cfg(test)]
mod tests;

pub use measurement::measure_push;
pub use scenario::{
    DEFAULT_QUEUE_CAPACITY,
    DEFAULT_SENDS,
    PushMeasurement,
    PushScenario,
    PushWorkload,
};
