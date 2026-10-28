//! Tests for pool, handle, lease, and shared-core lifetimes.

use std::{
    error::Error,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    client::pool::{
        PoolFairnessPolicy,
        PoolTaskChurnObserver,
        client_pool::{PoolSchedulerControl, WireframeClientPool},
        core::PoolCore,
        lease::PooledClientLease,
        test_support::{DropProbe, test_pool_slot},
    },
    serializer::BincodeSerializer,
};

type TestPool = WireframeClientPool<BincodeSerializer, (), ()>;

fn test_pool(drop_count: Option<Arc<AtomicUsize>>) -> TestPool {
    let mut slot = test_pool_slot(1, Duration::from_secs(30));
    if let Some(count) = &drop_count {
        slot.drop_probe = Some(DropProbe::new(Arc::clone(count)));
    }
    let mut core = PoolCore::new(vec![slot].into_boxed_slice());
    if let Some(count) = drop_count {
        core.drop_probe = Some(DropProbe::new(count));
    }

    TestPool {
        core: Arc::new(core),
        control: Arc::new(PoolSchedulerControl::new(PoolFairnessPolicy::Fifo, 8)),
    }
}

fn test_lease(pool: &TestPool) -> PooledClientLease<BincodeSerializer, (), ()> {
    let core = Arc::clone(&pool.core);
    let slot = core.slot(0).expect("the only pool slot should exist");
    let permit = slot
        .try_acquire_permit()
        .expect("the only pool slot should grant a permit");
    PooledClientLease::new(core, 0, permit, None)
        .expect("the selected slot should resolve in the pool core")
}

fn has_unexpected_eof(error: &(dyn Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(error) = source {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|io_error| io_error.kind() == io::ErrorKind::UnexpectedEof)
        {
            return true;
        }
        source = error.source();
    }
    false
}

async fn wait_for_capacity_wait(observer: &PoolTaskChurnObserver, message: &str) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while observer.snapshot().capacity_waits == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect(message);
}

#[tokio::test]
async fn pool_and_handle_drop_keep_lease_slot_alive_until_final_lease_drop() {
    let drops = Arc::new(AtomicUsize::new(0));
    let pool = test_pool(Some(Arc::clone(&drops)));
    let handle = pool.handle();
    let lease = test_lease(&pool);

    drop(pool);
    assert_eq!(drops.load(Ordering::Acquire), 0);
    assert!(lease.slot().is_some(), "lease must still resolve its slot");

    drop(handle);
    assert_eq!(drops.load(Ordering::Acquire), 0);
    assert!(lease.slot().is_some(), "lease remains the core's owner");

    drop(lease);
    assert_eq!(
        drops.load(Ordering::Acquire),
        2,
        "the core and its by-value slot must both drop with the final lease"
    );
}

#[tokio::test]
async fn close_rejects_new_and_queued_acquisitions_while_an_existing_lease_keeps_its_slot() {
    let pool = test_pool(None);
    let mut handle = pool.handle();
    let lease = test_lease(&pool);
    let observer = pool.task_churn_observer();
    let mut queued_handle = pool.handle();
    let queued_acquisition = tokio::spawn(async move { queued_handle.acquire().await });

    wait_for_capacity_wait(
        &observer,
        "the second acquisition should enter the wait path before close",
    )
    .await;

    pool.close().await;

    let new_result = handle.acquire().await;
    let Err(new_error) = new_result else {
        panic!("a closed pool must reject new leases");
    };
    assert!(
        has_unexpected_eof(&new_error),
        "a closed pool should report the disconnected UnexpectedEof error: {new_error}"
    );
    let queued_result = tokio::time::timeout(Duration::from_secs(1), queued_acquisition)
        .await
        .expect("pool close should complete the queued acquisition")
        .expect("the queued acquisition task should complete");
    let Err(queued_error) = queued_result else {
        panic!("a queued acquisition must not receive a lease after close");
    };
    assert!(
        has_unexpected_eof(&queued_error),
        "a queued acquisition should report the disconnected UnexpectedEof error: {queued_error}"
    );
    assert!(
        lease.slot().is_some(),
        "an existing lease must retain its selected slot through close"
    );
}

#[tokio::test]
async fn cancelled_queued_acquisition_releases_capacity_and_core_ownership() {
    let drops = Arc::new(AtomicUsize::new(0));
    let pool = test_pool(Some(Arc::clone(&drops)));
    let mut owner_handle = pool.handle();
    let lease = test_lease(&pool);
    let observer = pool.task_churn_observer();
    let mut queued_handle = pool.handle();
    let queued_acquisition = tokio::spawn(async move { queued_handle.acquire().await });

    wait_for_capacity_wait(
        &observer,
        "the acquisition should enter the capacity wait before cancellation",
    )
    .await;

    queued_acquisition.abort();
    let cancelled = tokio::time::timeout(Duration::from_secs(1), queued_acquisition)
        .await
        .expect("the cancelled acquisition task should stop promptly");
    let Err(join_error) = cancelled else {
        panic!("aborting the acquisition should cancel its task");
    };
    assert!(
        join_error.is_cancelled(),
        "the queued acquisition task should have been cancelled"
    );

    drop(lease);
    let later_lease = tokio::time::timeout(Duration::from_secs(1), owner_handle.acquire())
        .await
        .expect("capacity released after cancellation should wake a fresh acquisition")
        .expect("cancellation must not consume the released slot permit");
    drop(later_lease);
    drop(owner_handle);
    drop(pool);

    tokio::time::timeout(Duration::from_secs(1), async {
        while drops.load(Ordering::Acquire) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the cancelled race must not retain the pool core or its slot");
}
