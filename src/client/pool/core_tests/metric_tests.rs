//! Recorder-backed coverage for the acquisition metric wrappers.
//!
//! The metric tests in `crate::metrics::client_pool` call
//! `inc_pool_acquisition` directly, so they pass even if a wrapper stops
//! calling it or classifies an outcome wrongly. These tests capture what the
//! real wrappers record, which is what makes the telemetry trustworthy.

use std::sync::Arc;

use rstest::rstest;
use wireframe_testing::ObservabilityHandle;

use super::support::{TestCore, test_core};
use crate::client::{ClientError, pool::core::SlotPermit};

/// Drive the real acquisition wrappers and check the labels they emit.
#[rstest]
#[case::immediate_granted("granted", FastPathOutcome::Granted)]
#[case::immediate_exhausted("exhausted", FastPathOutcome::Exhausted)]
#[case::immediate_shutdown("shutdown", FastPathOutcome::Shutdown)]
#[test]
fn immediate_acquisition_records_its_outcome_label(
    #[case] expected: &str,
    #[case] outcome: FastPathOutcome,
) {
    let mut observability = ObservabilityHandle::new();
    // A slot's backing `bb8` pool builds a time interval eagerly, so slot
    // construction needs a reactor even though this path never awaits.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a current-thread runtime");
    metrics::with_local_recorder(observability.recorder(), || {
        runtime.block_on(async {
            let core = test_core(1);
            let _permit = observe_fast_path(&core, outcome);
        });
    });

    expect_pool_acquisition(&mut observability, "immediate", expected);
}

/// Drive the fast path into `outcome`, returning any permit it gained.
///
/// The permit is handed back rather than dropped here so the caller decides
/// the lifetime. Dropping it inside the async block would release capacity
/// before the recorder scope closes, which does not affect this metric but
/// makes the helper harder to reuse for exhaustion checks.
fn observe_fast_path(core: &TestCore, outcome: FastPathOutcome) -> Option<SlotPermit> {
    match outcome {
        FastPathOutcome::Granted => core.try_acquire_permit(),
        FastPathOutcome::Exhausted => {
            let permit = core.try_acquire_permit()?;
            assert!(
                core.try_acquire_permit().is_none(),
                "a held slot must not regrant"
            );
            Some(permit)
        }
        FastPathOutcome::Shutdown => {
            core.shutdown();
            assert!(core.try_acquire_permit().is_none(), "shutdown must reject");
            None
        }
    }
}

/// Which outcome the immediate fast path should observe in a metric test.
#[derive(Clone, Copy, Debug)]
enum FastPathOutcome {
    /// The slot granted a permit.
    Granted,
    /// Every slot was already leased.
    Exhausted,
    /// The core had been shut down.
    Shutdown,
}

/// The permit race reports three outcomes: a grant, a shutdown rejection, and
/// a slot error.
///
/// `exhausted` is deliberately absent. A slot that never frees is a wait, not
/// an immediate failure, so the race only ends when capacity arrives, the pool
/// shuts down, or a slot reports an error.
///
/// The counter is incremented when the acquisition future completes, not when
/// it is created, so each future must be polled inside the recorder scope. A
/// current-thread runtime keeps that polling on this thread, which is where
/// `with_local_recorder` installs the recorder.
#[rstest]
#[case::permit_race_granted("granted", PermitRaceOutcome::Granted)]
#[case::permit_race_shutdown("shutdown", PermitRaceOutcome::Shutdown)]
#[case::permit_race_error("error", PermitRaceOutcome::Error)]
#[test]
fn permit_race_records_its_outcome_label(
    #[case] expected: &str,
    #[case] outcome: PermitRaceOutcome,
) {
    let mut observability = ObservabilityHandle::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a current-thread runtime");

    let raced = metrics::with_local_recorder(observability.recorder(), || {
        runtime.block_on(async {
            let core = test_core(1);
            observe_permit_race(&core, outcome).await
        })
    });
    let expected_grant = matches!(outcome, PermitRaceOutcome::Granted);
    assert_eq!(
        raced.is_ok(),
        expected_grant,
        "outcome {outcome:?} should report a grant only when capacity exists"
    );
    expect_pool_acquisition(&mut observability, "permit_race", expected);
}

/// Drive the permit race into `outcome`, returning its classified result.
async fn observe_permit_race(
    core: &Arc<TestCore>,
    outcome: PermitRaceOutcome,
) -> Result<SlotPermit, ClientError> {
    match outcome {
        PermitRaceOutcome::Granted => core.acquire_slot_permit().await,
        PermitRaceOutcome::Shutdown => {
            core.shutdown();
            core.acquire_slot_permit().await
        }
        PermitRaceOutcome::Error => {
            // This is the only way to reach the wrapper's `error` branch: no
            // production path closes a slot's admission gate, so the race
            // cannot otherwise complete with a slot error while the core is
            // still live.
            core.slot(0)
                .expect("the only slot should exist")
                .close_admission();
            core.acquire_slot_permit().await
        }
    }
}

/// Which outcome the permit race should observe in a metric test.
#[derive(Clone, Copy, Debug)]
enum PermitRaceOutcome {
    /// A slot granted a permit.
    Granted,
    /// The core had been shut down.
    Shutdown,
    /// A slot reported an error while the core was still live.
    Error,
}

/// Assert the pool-acquisition counter recorded one attempt for these labels.
fn expect_pool_acquisition(observability: &mut ObservabilityHandle, mode: &str, outcome: &str) {
    observability.snapshot();
    observability
        .assert_counter(
            crate::metrics::POOL_ACQUISITIONS,
            [("mode", mode), ("outcome", outcome)],
            1,
        )
        .expect("the acquisition wrapper should record the expected label pair");
}
