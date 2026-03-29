//! Async tests for applying backpressure policies.

use std::{future::Future, io, time::Duration};

use super::super::backpressure::{MemoryPressureAction, apply_memory_pressure};

fn run_async_test<T>(future: impl Future<Output = T>) -> T {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => panic!("failed to build tokio runtime for backpressure tests: {error}"),
    };
    runtime.block_on(future)
}

#[test]
fn apply_memory_pressure_abort_returns_invalid_data() {
    run_async_test(async {
        match apply_memory_pressure(MemoryPressureAction::Abort, || {}).await {
            Ok(()) => panic!("Abort should return an error"),
            Err(err) => assert_eq!(
                err.kind(),
                io::ErrorKind::InvalidData,
                "expected InvalidData error kind, got {:?}: {err}",
                err.kind()
            ),
        }
    });
}

#[test]
fn apply_memory_pressure_pause_invokes_purge_closure() {
    run_async_test(async {
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
    });
}

#[test]
fn apply_memory_pressure_continue_is_noop() {
    run_async_test(async {
        let result = apply_memory_pressure(MemoryPressureAction::Continue, || {
            panic!("purge closure should not be called for Continue action");
        })
        .await;
        if let Err(error) = result {
            panic!("Continue should return Ok: {error}");
        }
    });
}
