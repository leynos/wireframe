//! Deterministic assertion helpers for reassembly-heavy tests.

mod assert_helpers;
mod fragment;
mod message;
mod message_error;

pub use fragment::{
    FragmentReassemblyErrorExpectation,
    FragmentReassemblySnapshot,
    assert_fragment_reassembly_absent,
    assert_fragment_reassembly_buffered_messages,
    assert_fragment_reassembly_completed_bytes,
    assert_fragment_reassembly_completed_len,
    assert_fragment_reassembly_error,
    assert_fragment_reassembly_evicted,
};
pub use message::{
    MessageAssemblySnapshot,
    assert_message_assembly_buffered_count,
    assert_message_assembly_completed,
    assert_message_assembly_completed_for_key,
    assert_message_assembly_error,
    assert_message_assembly_evicted,
    assert_message_assembly_incomplete,
    assert_message_assembly_total_buffered_bytes,
};
pub use message_error::MessageAssemblyErrorExpectation;
