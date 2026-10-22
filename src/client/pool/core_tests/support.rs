//! Shared fixtures for the client-pool core test modules.
//!
//! `core_tests` and `core_metric_tests` are both `#[path]`-included by
//! `pool::core`. Rust merges them into one `pool::core::tests` module, so
//! neither can reach the other's private items by name even though they are
//! siblings on disk. Defining the cores and their fixtures here gives both
//! modules a single included definition instead of drifting copies.

use std::{sync::Arc, time::Duration};

use rstest::fixture;

use crate::{
    client::pool::{core::PoolCore, test_support::test_pool_slot},
    serializer::BincodeSerializer,
};

/// The core type every client-pool core test drives.
pub(crate) type TestCore = PoolCore<BincodeSerializer, (), ()>;

/// Build a core of `slot_count` lazy slots without connecting.
pub(crate) fn test_core(slot_count: usize) -> Arc<TestCore> {
    let slots = (0..slot_count)
        .map(|_| test_pool_slot(1, Duration::from_secs(30)))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Arc::new(TestCore::new(slots))
}

#[fixture]
pub(crate) fn four_slot_core() -> Arc<TestCore> {
    let slot_count = 4;
    test_core(slot_count)
}

#[fixture]
pub(crate) fn two_slot_core() -> Arc<TestCore> {
    let slot_count = 2;
    test_core(slot_count)
}

#[fixture]
pub(crate) fn one_slot_core() -> Arc<TestCore> {
    let slot_count = 1;
    test_core(slot_count)
}
