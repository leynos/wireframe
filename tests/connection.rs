//! Integration tests for connection actor queue helpers and multi-packet handling.
//!
//! Scenarios are grouped by concern under the `connection/` module directory:
//! - Shared counters, harness configuration, and assertions (`support.rs`)
//! - Multi-packet forwarding, closed handling, and drains (`multi_packet.rs`)
//! - Closure-reason logging (`logs.rs`)
//! - Queue polling and lifecycle state (`state.rs`)
#![cfg(not(loom))]

#[path = "connection/logs.rs"]
mod logs;
#[path = "connection/multi_packet.rs"]
mod multi_packet;
#[path = "connection/state.rs"]
mod state;
#[path = "connection/support.rs"]
mod support;
