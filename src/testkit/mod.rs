//! Optional test utilities for downstream protocol crates.
//!
//! Enable the `testkit` feature to access helpers for:
//! - feeding partial frames or fragments into an in-process app;
//! - simulating slow readers and writers for back-pressure tests; and
//! - asserting message-assembly and fragment-reassembly outcomes without panicking.

mod fragment_drive;
mod partial_frame;
pub mod reassembly;
mod result;
mod slow_io;
mod support;

pub use fragment_drive::{
    drive_with_fragment_frames,
    drive_with_fragments,
    drive_with_fragments_mut,
    drive_with_fragments_with_capacity,
    drive_with_partial_fragments,
};
pub use partial_frame::{
    drive_with_partial_codec_frames,
    drive_with_partial_frames,
    drive_with_partial_frames_mut,
    drive_with_partial_frames_with_capacity,
};
pub use reassembly::{
    FragmentReassemblyErrorExpectation,
    FragmentReassemblySnapshot,
    MessageAssemblyErrorExpectation,
    MessageAssemblySnapshot,
    assert_fragment_reassembly_absent,
    assert_fragment_reassembly_buffered_messages,
    assert_fragment_reassembly_completed_bytes,
    assert_fragment_reassembly_completed_len,
    assert_fragment_reassembly_error,
    assert_fragment_reassembly_evicted,
    assert_message_assembly_buffered_count,
    assert_message_assembly_completed,
    assert_message_assembly_completed_for_key,
    assert_message_assembly_error,
    assert_message_assembly_evicted,
    assert_message_assembly_incomplete,
    assert_message_assembly_total_buffered_bytes,
};
pub use result::{TestError, TestResult};
pub use slow_io::{
    MAX_SLOW_IO_CAPACITY,
    SlowIoConfig,
    SlowIoPacing,
    drive_with_slow_codec_frames,
    drive_with_slow_codec_payloads,
    drive_with_slow_frames,
    drive_with_slow_payloads,
};
pub use support::TestSerializer;
