//! Unit and property coverage for connection identifier allocation and
//! lazy dead-entry cleanup in the session registry.

use std::sync::Arc;

use proptest::{prelude::*, test_runner::TestCaseError};

use super::{ConnectionId, SessionRegistry};
use crate::push::PushQueues;

/// A failed lookup must also evict the dead weak entry; retaining it
/// grows the registry until a later `prune`, which is the observable
/// difference the `strong_count() == 0` guard protects.
#[tokio::test]
async fn get_evicts_dead_entry_from_registry() {
    let registry = SessionRegistry::<u8>::default();
    let id = ConnectionId::new(1);

    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build PushQueues");
    registry.insert(id, &handle);
    drop(handle);
    drop(queues);

    assert!(registry.get(&id).is_none(), "dead handle must not upgrade");
    assert!(
        registry.entries.is_empty(),
        "failed lookup must evict the dead entry rather than leave it to prune"
    );
}

#[test]
fn connection_ids_start_at_one_and_increase() {
    let registry = SessionRegistry::<u8>::new();

    assert_eq!(registry.allocate_connection_id(), ConnectionId::new(1));
    assert_eq!(registry.allocate_connection_id(), ConnectionId::new(2));
    assert_eq!(registry.allocate_connection_id(), ConnectionId::new(3));
}

#[test]
fn default_registry_allocates_ids_from_one() {
    let registry = SessionRegistry::<u8>::default();

    assert_eq!(registry.allocate_connection_id(), ConnectionId::new(1));
}

// Every identifier a fresh registry hands out, however many are requested, is
// unique and strictly increasing from one. Strict increase is what makes
// `ConnectionId` a durable key: a value freed by a closed connection is never
// reused, so a stale identifier cannot address a later connection.
proptest! {
    #[test]
    fn allocation_is_unique_and_strictly_increasing(requested in 1usize..=512) {
        let registry = SessionRegistry::<u8>::new();
        let ids: Vec<u64> = (0..requested)
            .map(|_| registry.allocate_connection_id().as_u64())
            .collect();

        prop_assert_eq!(
            ids.first().copied(),
            Some(1),
            "the first identifier must be 1"
        );
        prop_assert!(
            ids.windows(2).all(|pair| pair[0] < pair[1]),
            "identifiers must increase strictly: {:?}",
            ids
        );
    }
}

// Concurrent producers share one allocator without duplicating an identifier
// or reordering one thread's own run. A per-thread run is a sequence a single
// connection allocates in order, so it must increase strictly even while other
// threads interleave. Sorting the combined runs then proves uniqueness across
// threads without depending on the interleaving that produced them.
proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn concurrent_allocation_keeps_ids_unique(
        threads in 1usize..=8,
        per_thread in 1usize..=32,
    ) {
        let registry = Arc::new(SessionRegistry::<u8>::new());
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let registry = Arc::clone(&registry);
                std::thread::spawn(move || {
                    (0..per_thread)
                        .map(|_| registry.allocate_connection_id().as_u64())
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        let mut all = Vec::with_capacity(threads * per_thread);
        for worker in workers {
            let run = worker
                .join()
                .map_err(|_| TestCaseError::fail("an allocation thread panicked"))?;
            prop_assert!(
                run.windows(2).all(|pair| pair[0] < pair[1]),
                "one thread's identifiers must increase strictly: {:?}",
                run
            );
            all.extend(run);
        }
        all.sort_unstable();

        prop_assert_eq!(all.len(), threads * per_thread);
        prop_assert_eq!(
            all.first().copied(),
            Some(1),
            "the lowest identifier must be 1"
        );
        prop_assert!(
            all.windows(2).all(|pair| pair[0] < pair[1]),
            "identifiers handed to different threads must be unique: {:?}",
            all
        );
    }
}
