//! Steps for fragment metadata behavioural tests.
use std::time::Duration;

use cucumber::{given, then, when};
use wireframe::{FragmentHeader, FragmentIndex, MessageId};

use crate::world::{FragmentWorld, TestResult};

#[given(expr = "a fragment series for message {int}")]
fn given_series(world: &mut FragmentWorld, message: u64) { world.start_series(message); }

#[given(expr = "the series expects fragment index {int}")]
fn given_series_expectation(world: &mut FragmentWorld, index: u32) -> TestResult {
    world.force_next_index(index)?;
    Ok(())
}

#[when(expr = "fragment {int} arrives marked non-final")]
fn when_fragment_non_final(world: &mut FragmentWorld, index: u32) -> TestResult {
    world.accept_fragment(index, false)?;
    Ok(())
}

#[when(expr = "fragment {int} arrives marked final")]
fn when_fragment_final(world: &mut FragmentWorld, index: u32) -> TestResult {
    world.accept_fragment(index, true)?;
    Ok(())
}

#[when(expr = "fragment {int} from message {int} arrives marked non-final")]
fn when_fragment_other_message(world: &mut FragmentWorld, index: u32, message: u64) -> TestResult {
    world.accept_fragment_from(message, index, false)?;
    Ok(())
}

#[then("the fragment completes the message")]
fn then_fragment_completes(world: &mut FragmentWorld) -> TestResult {
    world.assert_completion()?;
    Ok(())
}

#[then("the fragment is rejected as out-of-order")]
fn then_fragment_out_of_order(world: &mut FragmentWorld) -> TestResult {
    world.assert_index_mismatch()?;
    Ok(())
}

#[then("the fragment is rejected for the wrong message")]
fn then_fragment_wrong_message(world: &mut FragmentWorld) -> TestResult {
    world.assert_message_mismatch()?;
    Ok(())
}

#[then("the fragment is rejected for index overflow")]
fn then_fragment_overflow(world: &mut FragmentWorld) -> TestResult {
    world.assert_index_overflow()?;
    Ok(())
}

#[then("the fragment is rejected because the series is complete")]
fn then_fragment_complete(world: &mut FragmentWorld) -> TestResult {
    world.assert_series_complete_error()?;
    Ok(())
}

#[given(expr = "a fragmenter capped at {int} bytes per fragment")]
fn given_fragmenter(world: &mut FragmentWorld, max_payload: usize) -> TestResult {
    world.configure_fragmenter(max_payload)?;
    Ok(())
}

#[when(expr = "the fragmenter splits a payload of {int} bytes")]
fn when_fragmenter_splits(world: &mut FragmentWorld, len: usize) -> TestResult {
    world.fragment_payload(len)?;
    Ok(())
}

#[then(expr = "the fragmenter produces {int} fragments")]
fn then_fragment_count(world: &mut FragmentWorld, expected: usize) -> TestResult {
    world.assert_fragment_count(expected)?;
    Ok(())
}

#[then(expr = "fragment {int} carries {int} bytes")]
fn then_fragment_payload_len(world: &mut FragmentWorld, index: usize, len: usize) -> TestResult {
    world.assert_fragment_payload_len(index, len)?;
    Ok(())
}

#[then(expr = "fragment {int} is marked final")]
fn then_fragment_final(world: &mut FragmentWorld, index: usize) -> TestResult {
    world.assert_fragment_final_flag(index, true)?;
    Ok(())
}

#[then(expr = "fragment {int} is marked non-final")]
fn then_fragment_non_final(world: &mut FragmentWorld, index: usize) -> TestResult {
    world.assert_fragment_final_flag(index, false)?;
    Ok(())
}

#[then(expr = "the fragments use message id {int}")]
fn then_fragment_message_id(world: &mut FragmentWorld, message_id: u64) -> TestResult {
    world.assert_message_id(message_id)?;
    Ok(())
}

#[given(expr = "a reassembler allowing {int} bytes with a {int}-second reassembly timeout")]
fn given_reassembler(world: &mut FragmentWorld, max_bytes: usize, timeout_secs: u64) -> TestResult {
    world.configure_reassembler(max_bytes, timeout_secs)?;
    Ok(())
}

#[when(expr = "fragment {int} for message {int} with {int} bytes arrives marked non-final")]
fn when_reassembler_fragment_non_final(
    world: &mut FragmentWorld,
    index: u32,
    message: u64,
    len: usize,
) -> TestResult {
    let header = FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), false);
    world.push_fragment(header, len)?;
    Ok(())
}

#[when(expr = "fragment {int} for message {int} with {int} bytes arrives marked final")]
fn when_reassembler_fragment_final(
    world: &mut FragmentWorld,
    index: u32,
    message: u64,
    len: usize,
) -> TestResult {
    let header = FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), true);
    world.push_fragment(header, len)?;
    Ok(())
}

#[when(expr = "time advances by {int} seconds for reassembly")]
fn when_time_advances(world: &mut FragmentWorld, seconds: u64) -> TestResult {
    world.advance_time(Duration::from_secs(seconds))?;
    Ok(())
}

#[when("expired reassembly buffers are purged")]
fn when_reassembly_purged(world: &mut FragmentWorld) -> TestResult {
    world.purge_reassembly()?;
    Ok(())
}

#[then(expr = "the reassembler outputs a payload of {int} bytes")]
fn then_reassembled_len(world: &mut FragmentWorld, expected: usize) -> TestResult {
    world.assert_reassembled_len(expected)?;
    Ok(())
}

#[then("no message has been reassembled yet")]
fn then_no_reassembled_message(world: &mut FragmentWorld) -> TestResult {
    world.assert_no_reassembly()?;
    Ok(())
}

#[then("the reassembler reports a message-too-large error")]
fn then_reassembly_over_limit(world: &mut FragmentWorld) -> TestResult {
    world.assert_reassembly_over_limit()?;
    Ok(())
}

#[then("the reassembler reports an out-of-order fragment error")]
fn then_reassembly_out_of_order(world: &mut FragmentWorld) -> TestResult {
    world.assert_reassembly_out_of_order()?;
    Ok(())
}

#[then(expr = "the reassembler is buffering {int} messages")]
fn then_buffered_messages(world: &mut FragmentWorld, expected: usize) -> TestResult {
    world.assert_buffered_messages(expected)?;
    Ok(())
}

#[then(expr = "message {int} is evicted")]
fn then_message_evicted(world: &mut FragmentWorld, message: u64) -> TestResult {
    world.assert_evicted_message(message)?;
    Ok(())
}
