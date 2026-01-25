//! Module organisation for fragment transport integration tests.
//!
//! Splits tests by concern:
//! - `rejection`: Tests for malformed, duplicate, and out-of-order fragments
//! - `eviction`: Tests for reassembly timeout and eviction behaviour

pub mod eviction;
pub mod rejection;
