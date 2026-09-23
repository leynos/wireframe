//! Tests for connection preamble reading.
#![cfg(not(loom))]

#[path = "basic.rs"]
mod basic;
#[path = "callbacks.rs"]
mod callbacks;
#[path = "responses.rs"]
mod responses;
#[path = "support.rs"]
mod support;
#[path = "timeouts.rs"]
mod timeouts;
