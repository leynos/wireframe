//! Workloads and measurement for the push handle's send path.
//!
//! The push handle is the producer-facing half of a connection: application
//! code, background workers, and protocol hooks all enqueue frames through it.
//! Two distinct costs live here, and the issue asks for them separately:
//!
//! - the **uncontended** cost of awaiting a send into a queue a consumer is draining, split by
//!   priority because the actor polls the high-priority queue first; and
//! - the **overflow** cost of a non-blocking send against a full queue, which branches on
//!   [`PushPolicy`] and never awaits capacity.
//!
//! Rate limiting is disabled in every scenario. The default limiter is a leaky
//! bucket capped at 100 pushes per second, so leaving it enabled would make the
//! measured cost almost entirely `leaky_bucket` bookkeeping and sleeping rather
//! than anything about queue ownership.
//!
//! # Drop policies
//!
//! [`PushScenario::OverflowReturnError`] reports [`PushError::QueueFull`].
//! [`PushScenario::OverflowDrop`] and [`PushScenario::OverflowWarnAndDrop`]
//! report success and route the frame to the dead-letter queue instead.
//!
//! The two outcomes are not equally observable. Dropped frames are *routed* to
//! the DLQ, so a receiver on that channel sees each one; the handle's own
//! `dlq_drops` counter is `pub(crate)` and increments only when the DLQ itself
//! is full or closed, so it is neither reachable nor the right number from
//! outside the crate. [`measure_push`] therefore reports
//! [`PushMeasurement::dead_lettered`], drained from the DLQ receiver, which is
//! the black-box observation the issue's method asks for.
//!
//! # Layout
//!
//! [`scenario`] describes a workload and what it must report; [`measurement`]
//! drives it. The two send regimes live in their own submodules there, because
//! they are separate control flows — one awaits capacity, the other never does
//! — and mixing them would obscure which branch a change affected.

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
