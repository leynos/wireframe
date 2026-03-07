//! Scenario tests for slow reader and writer simulation.

use rstest_bdd_macros::scenario;

use crate::fixtures::slow_io_backpressure::*;

#[scenario(
    path = "tests/features/slow_io_backpressure.feature",
    name = "Slow writer delays request completion"
)]
fn slow_writer_delays(slow_io_backpressure_world: SlowIoBackpressureWorld) {
    let _ = slow_io_backpressure_world;
}

#[scenario(
    path = "tests/features/slow_io_backpressure.feature",
    name = "Slow reader delays response draining"
)]
fn slow_reader_delays(slow_io_backpressure_world: SlowIoBackpressureWorld) {
    let _ = slow_io_backpressure_world;
}

#[scenario(
    path = "tests/features/slow_io_backpressure.feature",
    name = "Combined slow reader and writer still round-trips correctly"
)]
fn combined_slow_io_round_trip(slow_io_backpressure_world: SlowIoBackpressureWorld) {
    let _ = slow_io_backpressure_world;
}
