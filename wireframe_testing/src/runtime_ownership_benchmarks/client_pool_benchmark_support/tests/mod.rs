//! Unit coverage for pool workload labels, rejection rules, and the counters.
//!
//! The cases are grouped by the claim each one discharges rather than by the
//! helper they call: `labels` pins what a workload's name advertises,
//! `uncontended` bounds the fast path, `contended` asserts the slow path is
//! genuinely entered and observed, and `sampling` checks the distribution
//! accounting over repeated bursts.

mod contended;
mod labels;
mod sampling;
mod uncontended;
