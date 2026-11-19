//! Unit and property coverage for connection identifier allocation,
//! publication ownership, and lazy dead-entry cleanup in the session registry.

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
    assert!(registry.insert(id, &handle));
    drop(handle);
    drop(queues);

    assert!(registry.get(&id).is_none(), "dead handle must not upgrade");
    assert!(
        registry.entries.is_empty(),
        "failed lookup must evict the dead entry rather than leave it to prune"
    );
}

/// Build a handle that keeps its registry entry live.
fn live_handle() -> (PushQueues<u8>, crate::push::PushHandle<u8>) {
    PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build PushQueues")
}

/// An application's own entry is never displaced by an allocation.
///
/// Identifier `1` is the first value a fresh registry hands out, so a manual
/// entry there is exactly the overlap the registration API has to tolerate:
/// the server connection moves past it and both handles stay addressable.
#[tokio::test]
async fn allocation_skips_an_identifier_a_live_entry_occupies() {
    let registry = SessionRegistry::<u8>::default();
    let manual_id = ConnectionId::new(1);
    let (_manual_queues, manual_handle) = live_handle();
    assert!(registry.insert(manual_id, &manual_handle));

    let allocated = registry.allocate_connection_id();

    assert_eq!(allocated, ConnectionId::new(2));
    assert!(
        registry.get(&manual_id).is_some(),
        "the manual entry must survive the allocation"
    );
}

/// A live handle cannot be replaced by a second publication.
///
/// Refusing the replacement is what stops a manual insert from taking over a
/// server connection's identifier after the connection registered.
#[tokio::test]
async fn insert_refuses_to_replace_a_live_entry() {
    let registry = SessionRegistry::<u8>::default();
    let id = ConnectionId::new(7);
    let (mut first_queues, first) = live_handle();
    let (_second_queues, second) = live_handle();

    assert!(registry.insert(id, &first));
    assert!(!registry.insert(id, &second));

    let published = registry.get(&id).expect("the first handle stays live");
    published
        .push_high_priority(11)
        .await
        .expect("the first lane has room");
    let (_, value) = first_queues.recv().await.expect("recv failed");
    assert_eq!(
        value, 11,
        "the published handle must still address the first connection"
    );
}

/// An application's own entry is never the one a claim publishes.
///
/// Identifier `1` is what a fresh registry hands out first, so a live manual
/// entry there is exactly the overlap the claim has to tolerate: it publishes
/// under `2` and both handles stay separately addressable. This is the outcome
/// the claim owes its callers; the window inside it is driven in
/// `a_competing_insert_in_the_claim_window_makes_it_retry`.
#[tokio::test]
async fn allocation_and_publication_claim_one_identifier() {
    let registry = SessionRegistry::<u8>::default();
    let manual_id = ConnectionId::new(1);
    let (mut manual_queues, manual_handle) = live_handle();
    assert!(registry.insert(manual_id, &manual_handle));
    let (mut server_queues, server_handle) = live_handle();

    let published = registry.allocate_and_publish(&server_handle);

    assert_eq!(published, ConnectionId::new(2));
    let addressed = registry
        .get(&published)
        .expect("the published entry must address the connection");
    addressed
        .push_high_priority(21)
        .await
        .expect("the published lane has room");
    assert_eq!(
        server_queues.recv().await.expect("recv failed").1,
        21,
        "the published identifier must address this connection"
    );
    let manual = registry
        .get(&manual_id)
        .expect("the manual entry must survive the publication");
    manual
        .push_high_priority(22)
        .await
        .expect("the manual lane has room");
    assert_eq!(
        manual_queues.recv().await.expect("recv failed").1,
        22,
        "the manual identifier must still address the application's handle"
    );
}

/// A competing insert in the claim's window makes it retry.
///
/// `claim_next` offers each candidate to its publisher before taking the next,
/// so the closure below is the claim's window: it lets a live manual entry take
/// the identifier the claim has just allocated, and only then attempts the
/// connection's own publication. That attempt is refused, the claim takes the
/// next identifier, and the connection ends up published under its own entry
/// while the manual one keeps the identifier it took. Dropping the retry
/// returns the taken identifier instead, which the assertion on `published`
/// catches before anything is pushed.
#[tokio::test]
async fn a_competing_insert_in_the_claim_window_makes_it_retry() {
    let registry = SessionRegistry::<u8>::default();
    let (mut manual_queues, manual_handle) = live_handle();
    let (mut connection_queues, connection_handle) = live_handle();
    let mut took_window = false;

    let published = registry.claim_next(|candidate| {
        if !took_window {
            took_window = true;
            assert!(
                registry.insert(candidate, &manual_handle),
                "the competing insert must take the freshly allocated candidate"
            );
        }
        registry.insert(candidate, &connection_handle)
    });

    let manual_id = ConnectionId::new(1);
    assert_eq!(
        published,
        ConnectionId::new(2),
        "the claim must retry past the identifier the competing insert took"
    );
    let addressed = registry
        .get(&published)
        .expect("the published entry must address the connection");
    addressed
        .push_high_priority(41)
        .await
        .expect("the published lane has room");
    assert_eq!(
        connection_queues.recv().await.expect("recv failed").1,
        41,
        "the published identifier must address this connection"
    );
    let manual = registry
        .get(&manual_id)
        .expect("the manual entry must survive the claim");
    manual
        .push_high_priority(42)
        .await
        .expect("the manual lane has room");
    assert_eq!(
        manual_queues.recv().await.expect("recv failed").1,
        42,
        "the identifier the manual insert took must still address its handle"
    );
}

/// Teardown removes only the entry the ending connection published.
///
/// An application may deliberately take an identifier over by removing the
/// entry and publishing its own handle there. Removing by identifier alone
/// would then deregister the newcomer when the connection ends, so removal is
/// conditional on the entry still holding the connection's own handle.
#[tokio::test]
async fn teardown_removes_only_the_entry_it_published() {
    let registry = SessionRegistry::<u8>::default();
    let id = ConnectionId::new(5);
    let (_ending_queues, ending) = live_handle();
    assert!(registry.insert(id, &ending));
    registry.remove(&id);
    let (_manual_queues, manual) = live_handle();
    assert!(registry.insert(id, &manual));

    assert!(
        !registry.remove_owned(id, &ending),
        "a later handle under the identifier must not be removed"
    );
    assert!(
        registry.get(&id).is_some(),
        "the application's handle must survive the ending connection's teardown"
    );
    assert!(
        registry.remove_owned(id, &manual),
        "removal must succeed while the entry holds the connection's handle"
    );
    assert!(registry.get(&id).is_none());
}

/// A dead entry does not block publication of a new handle.
#[tokio::test]
async fn insert_replaces_a_dead_entry() {
    let registry = SessionRegistry::<u8>::default();
    let id = ConnectionId::new(9);
    let (_dead_queues, dead) = live_handle();
    assert!(registry.insert(id, &dead));
    drop(dead);

    let (_live_queues, live) = live_handle();
    assert!(registry.insert(id, &live));
    assert!(registry.get(&id).is_some());
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
/// Whether every identifier is strictly greater than the one before it.
fn is_strictly_increasing(ids: &[u64]) -> bool {
    ids.windows(2)
        .all(|pair| matches!(pair, [earlier, later] if earlier < later))
}

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
            is_strictly_increasing(&ids),
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
                let shared = Arc::clone(&registry);
                std::thread::spawn(move || {
                    (0..per_thread)
                        .map(|_| shared.allocate_connection_id().as_u64())
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
                is_strictly_increasing(&run),
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
            is_strictly_increasing(&all),
            "identifiers handed to different threads must be unique: {:?}",
            all
        );
    }
}
