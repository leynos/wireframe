//! Tests for the `SessionRegistry`.
use rstest::{fixture, rstest};
use wireframe::{
    push::{PushHandle, PushQueues},
    session::{ConnectionId, SessionRegistry},
};

#[fixture]
#[allow(unused_braces)]
fn registry() -> SessionRegistry<u8> { SessionRegistry::default() }

#[fixture]
#[allow(unused_braces)]
fn push_setup() -> (PushQueues<u8>, PushHandle<u8>) { PushQueues::bounded(1, 1) }

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
    retrieved.push_high_priority(7).await.unwrap();
    let (_, val) = queues.recv().await.unwrap();
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
