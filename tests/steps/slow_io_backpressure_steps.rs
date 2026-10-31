//! Step definitions for slow-I/O behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::slow_io_backpressure::{
    CombinedDriveConfig,
    ReaderDriveConfig,
    SlowIoBackpressureWorld,
    TestResult,
};

#[given("a slow-io echo app with max frame length {max_frame_length:usize}")]
fn given_slow_io_app(
    slow_io_backpressure_world: &mut SlowIoBackpressureWorld,
    max_frame_length: usize,
) -> TestResult {
    slow_io_backpressure_world.configure_app(max_frame_length)
}

#[when(
    "a {payload_len:usize}-byte request is driven with slow writer pacing of {chunk_size:usize} \
     bytes every {delay_millis:u64} milliseconds"
)]
fn when_slow_writer(
    slow_io_backpressure_world: &mut SlowIoBackpressureWorld,
    payload_len: usize,
    chunk_size: usize,
    delay_millis: u64,
) -> TestResult {
    slow_io_backpressure_world.start_slow_writer(payload_len, chunk_size, delay_millis)
}

#[when("a slow reader drive is configured as {config}")]
fn when_slow_reader(
    slow_io_backpressure_world: &mut SlowIoBackpressureWorld,
    config: ReaderDriveConfig,
) -> TestResult {
    slow_io_backpressure_world.start_slow_reader(config)
}

#[when("a combined slow-io drive is configured as {config}")]
fn when_combined_slow_io(
    slow_io_backpressure_world: &mut SlowIoBackpressureWorld,
    config: CombinedDriveConfig,
) -> TestResult {
    slow_io_backpressure_world.start_combined(config)
}

#[then("the slow-io drive remains pending")]
fn then_pending(slow_io_backpressure_world: &mut SlowIoBackpressureWorld) -> TestResult {
    slow_io_backpressure_world.assert_pending()
}

#[when("slow-io virtual time advances by {millis:u64} milliseconds")]
fn when_advance_time(
    slow_io_backpressure_world: &mut SlowIoBackpressureWorld,
    millis: u64,
) -> TestResult {
    slow_io_backpressure_world.advance_millis(millis)
}

#[then("the slow-io drive completes with an echoed payload of {expected_len:usize} bytes")]
fn then_completed(
    slow_io_backpressure_world: &mut SlowIoBackpressureWorld,
    expected_len: usize,
) -> TestResult {
    slow_io_backpressure_world.assert_completed_payload_len(expected_len)
}
