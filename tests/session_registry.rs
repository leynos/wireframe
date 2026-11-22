//! Tests for the `SessionRegistry`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

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
    assert!(registry.insert(id, &handle));

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
    assert!(registry.insert(id, &handle));
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
    assert!(registry.insert(id, &handle));
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
    assert!(registry.insert(id1, &handle1));
    assert!(registry.insert(id2, &handle2));
    drop(handle1);

    let handles = registry.active_handles();
    assert_eq!(handles.len(), 1);
    let first = handles.first().expect("no active handles");
    assert_eq!(first.0, id2);
    Ok(())
}

/// Test that `prune` removes entries whose handles have been dropped and
/// retains entries whose handles are still alive.
#[rstest]
#[tokio::test]
async fn prune_removes_dead_entries(registry: SessionRegistry<u8>) -> TestResult<()> {
    let (_dead_queues, dead) = push_setup()?;
    let dead_id = ConnectionId::new(5);
    assert!(registry.insert(dead_id, &dead));
    drop(dead);

    let (_live_queues, live) = push_setup()?;
    let live_id = ConnectionId::new(6);
    assert!(registry.insert(live_id, &live));

    registry.prune();

    // `get` self-heals dead entries, so the `src/session_tests.rs` unit test
    // proves physical removal from the backing map; this checks the public
    // outcome.
    assert!(registry.get(&dead_id).is_none());
    assert!(registry.get(&live_id).is_some());
    assert_eq!(registry.active_ids(), vec![live_id]);
    Ok(())
}
