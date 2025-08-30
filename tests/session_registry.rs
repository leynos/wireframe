//! Tests for the `SessionRegistry`.
use rstest::{fixture, rstest};
use wireframe::{
    push::{PushHandle, PushQueues},
    session::{ConnectionId, SessionRegistry},
};

#[fixture]
#[expect(
    unused_braces,
    reason = "rustc false positive for single-line rstest fixtures"
)]
#[allow(unfulfilled_lint_expectations)]
fn registry() -> SessionRegistry<u8> { SessionRegistry::default() }

#[fixture]
#[expect(
    unused_braces,
    reason = "rustc false positive for single-line rstest fixtures"
)]
#[allow(unfulfilled_lint_expectations)]
fn push_setup() -> (PushQueues<u8>, PushHandle<u8>) {
    PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build PushQueues")
}

/// Test that handles can be retrieved whilst the connection remains alive.
#[rstest]
#[tokio::test]
async fn handle_retrieved_while_alive(
    registry: SessionRegistry<u8>,
    #[from(push_setup)] setup: (PushQueues<u8>, PushHandle<u8>),
) {
    let (mut queues, handle) = setup;
    let id = ConnectionId::new(42);
    registry.insert(id, &handle);

    let retrieved = registry.get(&id).expect("handle should be present");
    retrieved.push_high_priority(7).await.expect("push failed");
    let (_, val) = queues.recv().await.expect("recv failed");
    assert_eq!(val, 7);
}

/// Test that [`SessionRegistry::get`] returns `None` after the handle is dropped.
#[rstest]
#[tokio::test]
async fn get_returns_none_after_drop(
    registry: SessionRegistry<u8>,
    #[from(push_setup)] setup: (PushQueues<u8>, PushHandle<u8>),
) {
    let (_queues, handle) = setup;
    let id = ConnectionId::new(1);
    registry.insert(id, &handle);
    drop(handle);

    assert!(registry.get(&id).is_none());
}

/// Calling `get` should remove expired entries.
#[rstest]
#[tokio::test]
async fn get_prunes_dead_handle(
    registry: SessionRegistry<u8>,
    #[from(push_setup)] setup: (PushQueues<u8>, PushHandle<u8>),
) {
    let (_queues, handle) = setup;
    let id = ConnectionId::new(11);
    registry.insert(id, &handle);
    drop(handle);

    assert!(registry.get(&id).is_none());
    assert!(!registry.active_ids().contains(&id));
}

/// `active_handles` returns only live sessions.
#[rstest]
#[tokio::test]
async fn active_handles_lists_live_connections(
    registry: SessionRegistry<u8>,
    #[from(push_setup)] setup1: (PushQueues<u8>, PushHandle<u8>),
    #[from(push_setup)] setup2: (PushQueues<u8>, PushHandle<u8>),
) {
    let (_queues1, handle1) = setup1;
    let (_queues2, handle2) = setup2;
    let id1 = ConnectionId::new(21);
    let id2 = ConnectionId::new(22);
    registry.insert(id1, &handle1);
    registry.insert(id2, &handle2);
    drop(handle1);

    let handles = registry.active_handles();
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].0, id2);
}

/// Test that `prune` removes entries whose handles have been dropped.
#[rstest]
#[tokio::test]
async fn prune_removes_dead_entries(
    registry: SessionRegistry<u8>,
    #[from(push_setup)] setup: (PushQueues<u8>, PushHandle<u8>),
) {
    let (_queues, handle) = setup;
    let id = ConnectionId::new(5);
    registry.insert(id, &handle);
    drop(handle);
    registry.prune();

    assert!(registry.get(&id).is_none());
}
