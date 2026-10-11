//! Workloads and measurement for the connection actor's event loop.
//!
//! The actor loop is the hot path Epic #635 proposes to re-topologize, so its
//! baseline must describe how many queued frames actually travelled through the
//! loop, under which cancellation regime, and with what queue capacities. Those
//! three facts are workload parameters here rather than constants baked into a
//! benchmark binary, so the same scenario keeps working after the refactor and
//! can be pointed at an old and a new implementation unchanged.
//!
//! Frames are [`Envelope`] values rather than the crate's test-only `u8` frame.
//! `Envelope` implements [`Packet`](wireframe::app::Packet) without any feature
//! gate, so this helper works from a plain `wireframe` dependency and exercises
//! the same frame type production connections carry.
//!
//! # Cancellation regimes
//!
//! [`CancellationMode`] distinguishes the three behaviours the issue asks for.
//! They are not three timings of one scenario; they are three different control
//! flows through the loop, and the frame counts they produce differ by design:
//!
//! - [`CancellationMode::Disabled`] never cancels. The actor terminates because every producer has
//!   dropped, so the queues report closure.
//! - [`CancellationMode::CancelledBeforeRun`] cancels before [`run`] is called. The actor aborts
//!   immediately and emits nothing.
//! - [`CancellationMode::CancelledDuringDrain`] preloads every frame, then cancels from inside the
//!   output hook on the first emitted frame. Shutdown is therefore observed *while* the loop is
//!   draining, which is the regime that proves the loop neither short-circuits on shutdown nor
//!   abandons frames already buffered in the high- and low-priority queues.
//!
//! [`run`]: wireframe::connection::ConnectionActor::run
//!
//! # Preconditions
//!
//! Every frame a workload names must fit in the queues, so
//! `frames <= 2 * queue_capacity`. A workload that violates this is rejected by
//! [`measure_actor_drain`] rather than measured, because an over-capacity
//! workload would make the preload block on back-pressure and silently change
//! which regime was measured.
//!
//! # Layout
//!
//! [`workload`] holds the scenario as data and [`measurement`] drives it, for
//! the same reason as the other scopes: the workload definition is the contract
//! a before/after comparison rests on, so it must not move when the ownership
//! topology does.

mod measurement;
mod workload;

#[cfg(test)]
mod tests;

pub use measurement::measure_actor_drain;
pub use workload::{
    ActorMeasurement,
    ActorWorkload,
    CancellationMode,
    DEFAULT_FRAME_COUNT,
    DEFAULT_QUEUE_CAPACITY,
};
