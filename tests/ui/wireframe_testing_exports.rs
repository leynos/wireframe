//! Check ordinary-build exports from `wireframe_testing`.
//! This fixture keeps its test-support API under compile-time coverage.

pub use wireframe_testing::client_pair;
pub use wireframe_testing::runtime_ownership_benchmarks;
pub use wireframe_testing::wait_for_listener_release;
pub use wireframe_testing::{
    WireframePair,
    spawn_wireframe_pair,
    spawn_wireframe_pair_default,
};

fn main() {}
