//! Scenario tests for multi-packet responses.

use rstest_bdd_macros::scenario;

use crate::fixtures::multi_packet::*;

#[scenario(
    path = "tests/features/multi_packet.feature",
    name = "Response::with_channel streams frames sequentially"
)]
fn multi_packet_streaming(multi_packet_world: MultiPacketWorld) { let _ = multi_packet_world; }

#[scenario(
    path = "tests/features/multi_packet.feature",
    name = "no messages are emitted from a multi-packet response"
)]
fn multi_packet_empty(multi_packet_world: MultiPacketWorld) { let _ = multi_packet_world; }

#[scenario(
    path = "tests/features/multi_packet.feature",
    name = "Channel capacity overflow"
)]
fn multi_packet_overflow(multi_packet_world: MultiPacketWorld) { let _ = multi_packet_world; }
