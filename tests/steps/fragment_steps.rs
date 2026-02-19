//! Step definitions for fragment metadata behavioural tests.

use std::time::Duration;

use rstest_bdd_macros::{given, then, when};
use wireframe::fragment::{FragmentHeader, FragmentIndex, MessageId};

use crate::fixtures::fragment::{FragmentWorld, TestResult};

#[given("a fragment series for message {message:u64}")]
fn given_series(fragment_world: &mut FragmentWorld, message: u64) {
    fragment_world.start_series(message);
}

#[given("the series expects fragment index {index:u32}")]
fn given_series_expectation(fragment_world: &mut FragmentWorld, index: u32) -> TestResult {
    fragment_world.force_next_index(index)?;
    Ok(())
}

#[when("fragment {index:u32} arrives marked non-final")]
fn when_fragment_non_final(fragment_world: &mut FragmentWorld, index: u32) -> TestResult {
    fragment_world.accept_fragment(index, false)?;
    Ok(())
}

#[when("fragment {index:u32} arrives marked final")]
fn when_fragment_final(fragment_world: &mut FragmentWorld, index: u32) -> TestResult {
    fragment_world.accept_fragment(index, true)?;
    Ok(())
}

#[when("fragment {index:u32} from message {message:u64} arrives marked non-final")]
fn when_fragment_other_message(
    fragment_world: &mut FragmentWorld,
    index: u32,
    message: u64,
) -> TestResult {
    fragment_world.accept_fragment_from(message, index, false)?;
    Ok(())
}

#[then("the fragment completes the message")]
fn then_fragment_completes(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_completion()?;
    Ok(())
}

#[then("the fragment is rejected as out-of-order")]
fn then_fragment_out_of_order(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_index_mismatch()?;
    Ok(())
}

#[then("the fragment is rejected for the wrong message")]
fn then_fragment_wrong_message(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_message_mismatch()?;
    Ok(())
}

#[then("the fragment is rejected for index overflow")]
fn then_fragment_overflow(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_index_overflow()?;
    Ok(())
}

#[then("the fragment is rejected because the series is complete")]
fn then_fragment_complete(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_series_complete_error()?;
    Ok(())
}

#[given("a fragmenter capped at {max_payload:usize} bytes per fragment")]
fn given_fragmenter(fragment_world: &mut FragmentWorld, max_payload: usize) -> TestResult {
    fragment_world.configure_fragmenter(max_payload)?;
    Ok(())
}

#[when("the fragmenter splits a payload of {len:usize} bytes")]
fn when_fragmenter_splits(fragment_world: &mut FragmentWorld, len: usize) -> TestResult {
    fragment_world.fragment_payload(len)?;
    Ok(())
}

#[then("the fragmenter produces {expected:usize} fragments")]
fn then_fragment_count(fragment_world: &mut FragmentWorld, expected: usize) -> TestResult {
    fragment_world.assert_fragment_count(expected)?;
    Ok(())
}

#[then("fragment {index:usize} carries {len:usize} bytes")]
fn then_fragment_payload_len(
    fragment_world: &mut FragmentWorld,
    index: usize,
    len: usize,
) -> TestResult {
    fragment_world.assert_fragment_payload_len(index, len)?;
    Ok(())
}

#[then("fragment {index:usize} is marked final")]
fn then_fragment_final(fragment_world: &mut FragmentWorld, index: usize) -> TestResult {
    fragment_world.assert_fragment_final_flag(index, true)?;
    Ok(())
}

#[then("fragment {index:usize} is marked non-final")]
fn then_fragment_non_final(fragment_world: &mut FragmentWorld, index: usize) -> TestResult {
    fragment_world.assert_fragment_final_flag(index, false)?;
    Ok(())
}

#[then("the fragments use message id {message_id:u64}")]
fn then_fragment_message_id(fragment_world: &mut FragmentWorld, message_id: u64) -> TestResult {
    fragment_world.assert_message_id(message_id)?;
    Ok(())
}

#[given(
    "a reassembler allowing {max_bytes:usize} bytes with a {timeout_secs:u64}-second reassembly \
     timeout"
)]
fn given_reassembler(
    fragment_world: &mut FragmentWorld,
    max_bytes: usize,
    timeout_secs: u64,
) -> TestResult {
    fragment_world.configure_reassembler(max_bytes, timeout_secs)?;
    Ok(())
}

#[when(
    "fragment {index:u32} for message {message:u64} with {len:usize} bytes arrives marked \
     non-final"
)]
fn when_reassembler_fragment_non_final(
    fragment_world: &mut FragmentWorld,
    index: u32,
    message: u64,
    len: usize,
) -> TestResult {
    let header = FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), false);
    fragment_world.push_fragment(header, len)?;
    Ok(())
}

#[when(
    "fragment {index:u32} for message {message:u64} with {len:usize} bytes arrives marked final"
)]
#[when("fragment {index:u32} for message {message:u64} with {len:usize} byte arrives marked final")]
fn when_reassembler_fragment_final(
    fragment_world: &mut FragmentWorld,
    index: u32,
    message: u64,
    len: usize,
) -> TestResult {
    let header = FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), true);
    fragment_world.push_fragment(header, len)?;
    Ok(())
}

#[when("time advances by {seconds:u64} seconds for reassembly")]
fn when_time_advances(fragment_world: &mut FragmentWorld, seconds: u64) -> TestResult {
    fragment_world.advance_time(Duration::from_secs(seconds))?;
    Ok(())
}

#[when("expired reassembly buffers are purged")]
fn when_reassembly_purged(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.purge_reassembly()?;
    Ok(())
}

#[then("the reassembler outputs a payload of {expected:usize} bytes")]
fn then_reassembled_len(fragment_world: &mut FragmentWorld, expected: usize) -> TestResult {
    fragment_world.assert_reassembled_len(expected)?;
    Ok(())
}

#[then("no message has been reassembled yet")]
fn then_no_reassembled_message(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_no_reassembly()?;
    Ok(())
}

#[then("the reassembler reports a message-too-large error")]
fn then_reassembly_over_limit(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_reassembly_over_limit()?;
    Ok(())
}

#[then("the reassembler reports an out-of-order fragment error")]
fn then_reassembly_out_of_order(fragment_world: &mut FragmentWorld) -> TestResult {
    fragment_world.assert_reassembly_out_of_order()?;
    Ok(())
}

fn assert_buffered_messages(fragment_world: &mut FragmentWorld, expected: usize) -> TestResult {
    fragment_world.assert_buffered_messages(expected)?;
    Ok(())
}

#[then("the reassembler is buffering {expected:usize} message")]
fn then_buffered_message(fragment_world: &mut FragmentWorld, expected: usize) -> TestResult {
    assert_buffered_messages(fragment_world, expected)
}

#[then("the reassembler is buffering {expected:usize} messages")]
fn then_buffered_messages(fragment_world: &mut FragmentWorld, expected: usize) -> TestResult {
    assert_buffered_messages(fragment_world, expected)
}

#[then("message {message:u64} is evicted")]
fn then_message_evicted(fragment_world: &mut FragmentWorld, message: u64) -> TestResult {
    fragment_world.assert_evicted_message(message)?;
    Ok(())
}
