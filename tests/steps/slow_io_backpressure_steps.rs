//! Step definitions for slow-I/O behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::slow_io_backpressure::{
    CombinedDriveConfig,
    ReaderDriveConfig,
    SlowIoBackpressureWorld,
    TestResult,
};

fn world(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
) -> TestResult<&mut SlowIoBackpressureWorld> {
    slow_io_backpressure_world
        .as_mut()
        .map_err(|error| format!("slow-io fixture setup failed: {error}").into())
}

#[given("a slow-io echo app with max frame length {max_frame_length:usize}")]
fn given_slow_io_app(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
    max_frame_length: usize,
) -> TestResult {
    world(slow_io_backpressure_world)?.configure_app(max_frame_length)
}

#[when(
    "a {payload_len:usize}-byte request is driven with slow writer pacing of {chunk_size:usize} \
     bytes every {delay_millis:u64} milliseconds"
)]
fn when_slow_writer(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
    payload_len: usize,
    chunk_size: usize,
    delay_millis: u64,
) -> TestResult {
    world(slow_io_backpressure_world)?.start_slow_writer(payload_len, chunk_size, delay_millis)
}

#[when("a slow reader drive is configured as {config}")]
fn when_slow_reader(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
    config: ReaderDriveConfig,
) -> TestResult {
    world(slow_io_backpressure_world)?.start_slow_reader(config)
}

#[when("a combined slow-io drive is configured as {config}")]
fn when_combined_slow_io(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
    config: CombinedDriveConfig,
) -> TestResult {
    world(slow_io_backpressure_world)?.start_combined(config)
}

#[then("the slow-io drive remains pending")]
fn then_pending(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
) -> TestResult {
    world(slow_io_backpressure_world)?.assert_pending()
}

#[when("slow-io virtual time advances by {millis:u64} milliseconds")]
fn when_advance_time(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
    millis: u64,
) -> TestResult {
    world(slow_io_backpressure_world)?.advance_millis(millis)
}

#[then("the slow-io drive completes with an echoed payload of {expected_len:usize} bytes")]
fn then_completed(
    slow_io_backpressure_world: &mut TestResult<SlowIoBackpressureWorld>,
    expected_len: usize,
) -> TestResult {
    world(slow_io_backpressure_world)?.assert_completed_payload_len(expected_len)
}
