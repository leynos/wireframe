//! Tests for fixed slot storage and index-based acquisition.

use std::{sync::Arc, time::Duration};

use crate::{
    client::pool::{core::PoolCore, lease::PooledClientLease, test_support::test_pool_slot},
    serializer::BincodeSerializer,
};

type TestCore = PoolCore<BincodeSerializer, (), ()>;

#[tokio::test]
async fn acquired_indices_continue_to_resolve_the_same_by_value_slots() {
    let slots = (0..4)
        .map(|_| test_pool_slot(1, Duration::from_secs(30)))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let core = Arc::new(TestCore::new(slots));
    let slot_addresses = core
        .slots
        .iter()
        .map(std::ptr::from_ref)
        .collect::<Vec<_>>();

    let leases = (0..4)
        .map(|_| {
            let (slot_index, permit) = core
                .try_acquire_permit()
                .expect("each fixed slot should grant one permit");
            let lease = PooledClientLease::new(Arc::clone(&core), slot_index, permit, None)
                .expect("selected index should resolve in the shared core");
            (slot_index, lease)
        })
        .collect::<Vec<_>>();

    for (slot_index, lease) in &leases {
        let selected_slot = lease
            .slot()
            .expect("lease index should keep resolving while the core is alive");
        assert_eq!(
            std::ptr::from_ref(selected_slot),
            *slot_addresses
                .get(*slot_index)
                .expect("each acquired lease should have a recorded slot address"),
            "the index must continue to identify the original slot"
        );
    }
}

#[tokio::test]
async fn lease_construction_rejects_an_out_of_range_slot_index() {
    let core = Arc::new(TestCore::new(
        vec![test_pool_slot(1, Duration::from_secs(30))].into_boxed_slice(),
    ));
    let slot = core.slot(0).expect("the only slot should exist");
    let permit = slot
        .try_acquire_permit()
        .expect("the only slot should grant a permit");

    assert!(
        PooledClientLease::new(core, 1, permit, None).is_none(),
        "a lease must not be constructed with an invalid slot index"
    );
}

#[tokio::test]
async fn shutdown_before_notification_first_poll_is_not_lost() {
    let core = TestCore::new(vec![test_pool_slot(1, Duration::from_secs(30))].into_boxed_slice());
    let notification = core.shutdown_notified();

    core.shutdown();

    tokio::time::timeout(Duration::from_secs(1), notification)
        .await
        .expect("shutdown wait should observe shutdown before its first poll");
}
