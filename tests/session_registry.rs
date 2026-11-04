//! Tests for the `SessionRegistry`.
#![cfg(not(loom))]

use rstest::{fixture, rstest};
use wireframe::{
    push::{PushConfigError, PushHandle, PushQueues},
    session::{ConnectionId, SessionRegistry},
};

mod support;
use wireframe_testing::TestResult;

#[fixture]
fn registry() -> SessionRegistry<u8> {
    // Fixtures use the default registry to minimize setup noise.
    SessionRegistry::default()
}

fn push_setup() -> Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError> {
    support::builder().build()
}

/// Test that handles can be retrieved whilst the connection remains alive.
#[rstest]
#[tokio::test]
async fn handle_retrieved_while_alive(registry: SessionRegistry<u8>) -> TestResult<()> {
    let (mut queues, handle) = push_setup()?;
    let id = ConnectionId::new(42);
    registry.insert(id, &handle);

    let retrieved = registry.get(&id).expect("handle should be present");
    retrieved.push_high_priority(7).await?;
    let (_, val) = queues.recv().await.expect("recv failed");
    assert_eq!(val, 7);
    Ok(())
}

/// Test that [`SessionRegistry::get`] returns `None` after the handle is dropped.
#[rstest]
#[tokio::test]
async fn get_returns_none_after_drop(registry: SessionRegistry<u8>) -> TestResult<()> {
    let (_queues, handle) = push_setup()?;
    let id = ConnectionId::new(1);
    registry.insert(id, &handle);
    drop(handle);

    assert!(registry.get(&id).is_none());
    Ok(())
}

/// Calling `get` should remove expired entries.
#[rstest]
#[tokio::test]
async fn get_prunes_dead_handle(registry: SessionRegistry<u8>) -> TestResult<()> {
    let (_queues, handle) = push_setup()?;
    let id = ConnectionId::new(11);
    registry.insert(id, &handle);
    drop(handle);

    assert!(registry.get(&id).is_none());
    assert!(!registry.active_ids().contains(&id));
    Ok(())
}

/// `active_handles` returns only live sessions.
#[rstest]
#[tokio::test]
async fn active_handles_lists_live_connections(registry: SessionRegistry<u8>) -> TestResult<()> {
    let (_queues1, handle1) = push_setup()?;
    let (_queues2, handle2) = push_setup()?;
    let id1 = ConnectionId::new(21);
    let id2 = ConnectionId::new(22);
    registry.insert(id1, &handle1);
    registry.insert(id2, &handle2);
    drop(handle1);

    let handles = registry.active_handles();
    assert_eq!(handles.len(), 1);
    let first = handles.first().expect("no active handles");
    assert_eq!(first.0, id2);
    Ok(())
}

/// Test that `prune` removes entries whose handles have been dropped.
#[rstest]
#[tokio::test]
async fn prune_removes_dead_entries(registry: SessionRegistry<u8>) -> TestResult<()> {
    let (_queues, handle) = push_setup()?;
    let id = ConnectionId::new(5);
    registry.insert(id, &handle);
    drop(handle);
    registry.prune();

    assert!(registry.get(&id).is_none());
    Ok(())
}
