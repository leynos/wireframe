//! Tests for the `SessionRegistry`.
use rstest::rstest;
use wireframe::{
    push::PushQueues,
    session::{ConnectionId, SessionRegistry},
};

#[rstest]
#[tokio::test]
async fn handle_retrieved_while_alive() {
    let registry: SessionRegistry<u8> = SessionRegistry::default();
    let (mut queues, handle) = PushQueues::bounded(1, 1);
    let id = ConnectionId::from(42);
    registry.insert(id, &handle);

    let retrieved = registry.get(&id).expect("handle should be present");
    retrieved.push_high_priority(7).await.unwrap();
    let (_, val) = queues.recv().await.unwrap();
    assert_eq!(val, 7);
}

#[rstest]
#[tokio::test]
async fn get_returns_none_after_drop() {
    let registry: SessionRegistry<u8> = SessionRegistry::default();
    let (_queues, handle) = PushQueues::bounded(1, 1);
    let id = ConnectionId::from(1);
    registry.insert(id, &handle);
    drop(handle);

    assert!(registry.get(&id).is_none());
}

#[rstest]
#[tokio::test]
async fn prune_removes_dead_entries() {
    let registry: SessionRegistry<u8> = SessionRegistry::default();
    let (_queues, handle) = PushQueues::bounded(1, 1);
    let id = ConnectionId::from(5);
    registry.insert(id, &handle);
    drop(handle);
    registry.prune();

    assert!(registry.get(&id).is_none());
}
