//! Steps for fragment metadata behavioural tests.
use cucumber::{given, then, when};

use crate::world::FragmentWorld;

#[given(expr = "a fragment series for message {int}")]
fn given_series(world: &mut FragmentWorld, message: u64) { world.start_series(message); }

#[given(expr = "the series expects fragment index {int}")]
fn given_series_expectation(world: &mut FragmentWorld, index: u32) {
    world.force_next_index(index);
}

#[when(expr = "fragment {int} arrives marked non-final")]
fn when_fragment_non_final(world: &mut FragmentWorld, index: u32) {
    world.accept_fragment(index, false);
}

#[when(expr = "fragment {int} arrives marked final")]
fn when_fragment_final(world: &mut FragmentWorld, index: u32) {
    world.accept_fragment(index, true);
}

#[when(expr = "fragment {int} from message {int} arrives marked non-final")]
fn when_fragment_other_message(world: &mut FragmentWorld, index: u32, message: u64) {
    world.accept_fragment_from(message, index, false);
}

#[then("the fragment completes the message")]
fn then_fragment_completes(world: &mut FragmentWorld) { world.assert_completion(); }

#[then("the fragment is rejected as out-of-order")]
fn then_fragment_out_of_order(world: &mut FragmentWorld) { world.assert_index_mismatch(); }

#[then("the fragment is rejected for the wrong message")]
fn then_fragment_wrong_message(world: &mut FragmentWorld) { world.assert_message_mismatch(); }

#[then("the fragment is rejected for index overflow")]
fn then_fragment_overflow(world: &mut FragmentWorld) { world.assert_index_overflow(); }

#[then("the fragment is rejected because the series is complete")]
fn then_fragment_complete(world: &mut FragmentWorld) { world.assert_series_complete_error(); }

#[given(expr = "a fragmenter capped at {int} bytes per fragment")]
fn given_fragmenter(world: &mut FragmentWorld, max_payload: usize) {
    world.configure_fragmenter(max_payload);
}

#[when(expr = "the fragmenter splits a payload of {int} bytes")]
fn when_fragmenter_splits(world: &mut FragmentWorld, len: usize) { world.fragment_payload(len); }

#[then(expr = "the fragmenter produces {int} fragments")]
fn then_fragment_count(world: &mut FragmentWorld, expected: usize) {
    world.assert_fragment_count(expected);
}

#[then(expr = "fragment {int} carries {int} bytes")]
fn then_fragment_payload_len(world: &mut FragmentWorld, index: usize, len: usize) {
    world.assert_fragment_payload_len(index, len);
}

#[then(expr = "fragment {int} is marked final")]
fn then_fragment_final(world: &mut FragmentWorld, index: usize) {
    world.assert_fragment_final_flag(index, true);
}

#[then(expr = "fragment {int} is marked non-final")]
fn then_fragment_non_final(world: &mut FragmentWorld, index: usize) {
    world.assert_fragment_final_flag(index, false);
}

#[then(expr = "the fragments use message id {int}")]
fn then_fragment_message_id(world: &mut FragmentWorld, message_id: u64) {
    world.assert_message_id(message_id);
}
