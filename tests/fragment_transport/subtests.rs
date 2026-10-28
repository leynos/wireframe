//! Module organization for fragment transport integration tests.
//!
//! Splits tests by concern:
//! - `rejection`: Tests for malformed and out-of-order fragments
//! - `eviction`: Tests for reassembly timeout and eviction behaviour

#[path = "eviction.rs"]
pub mod eviction;
#[path = "rejection.rs"]
pub mod rejection;

#[path = "aggregate_cap.rs"]
pub mod aggregate_cap;
