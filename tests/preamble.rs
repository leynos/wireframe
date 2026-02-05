#![cfg_attr(loom, allow(missing_docs))]
#![cfg(not(loom))]
//! Tests for connection preamble reading.

mod common;

#[path = "preamble/basic.rs"]
mod basic;
#[path = "preamble/callbacks.rs"]
mod callbacks;
#[path = "preamble/responses.rs"]
mod responses;
#[path = "preamble/support.rs"]
mod support;
#[path = "preamble/timeouts.rs"]
mod timeouts;
