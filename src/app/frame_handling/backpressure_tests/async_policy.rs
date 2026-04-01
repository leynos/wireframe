//! Async tests for applying backpressure policies.

use std::{io, time::Duration};

use super::super::backpressure::{MemoryPressureAction, apply_memory_pressure};

#[tokio::test]
async fn apply_memory_pressure_abort_returns_invalid_data() {
    match apply_memory_pressure(MemoryPressureAction::Abort, || {}).await {
        Ok(()) => panic!("Abort should return an error"),
        Err(err) => assert_eq!(
            err.kind(),
            io::ErrorKind::InvalidData,
            "expected InvalidData error kind, got {:?}: {err}",
            err.kind()
        ),
    }
}

#[tokio::test]
async fn apply_memory_pressure_pause_invokes_purge_closure() {
    tokio::time::pause();

    let mut purge_called = false;
    let result = apply_memory_pressure(
        MemoryPressureAction::Pause(Duration::from_millis(1)),
        || purge_called = true,
    )
    .await;
    if let Err(error) = result {
        panic!("Pause should return Ok: {error}");
    }
    assert!(
        purge_called,
        "expected purge closure to be called during Pause action"
    );
}

#[tokio::test]
async fn apply_memory_pressure_continue_is_noop() {
    let result = apply_memory_pressure(MemoryPressureAction::Continue, || {
        panic!("purge closure should not be called for Continue action");
    })
    .await;
    if let Err(error) = result {
        panic!("Continue should return Ok: {error}");
    }
}
