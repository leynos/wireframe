//! Scenario test functions for rstest-bdd.
//!
//! Each scenario from the `.feature` files has a corresponding `#[scenario]`
//! test function here.

// Load step definitions first so compile-time validation can see them.
#[path = "../steps/mod.rs"]
mod steps;

mod client_runtime_scenarios;
mod codec_stateful_scenarios;
mod correlation_scenarios;
mod multi_packet_scenarios;
mod panic_scenarios;
mod request_parts_scenarios;
mod stream_end_scenarios;
