//! The contended burst: every contender racing for capacity behind a barrier.
//!
//! This is the regime that intermittently stalls, so it is bounded by a timeout
//! that converts a hang into a reported error. The timeout is a safety net, not
//! a performance threshold.

use std::{
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::Barrier;

use super::acquire_round;
use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::workload::{
    CONTENDED_ROUNDS_PER_CONTENDER,
    PoolWorkload,
};

/// Upper bound on a single contended burst.
///
/// This is a *safety net*, not a performance threshold, and it exists because
/// measurement found a pre-existing deadlock rather than a slow path. Sustained
/// contention can leave every tokio worker parked with waiters still queued, so
/// a burst never completes and the call site hangs forever. Observed on the
/// named baseline machine at burst 272, 808, 814, 2361, 2739, 4233, 4797,
/// 10727, and 18069 across both fairness policies, with and without the
/// per-iteration pool rebuild, and with fresh and reused handles — so the
/// trigger is contention itself, not the harness shape.
///
/// **That deadlock no longer reproduces.** Those bursts were recorded against
/// the pre-#535 scheduler, and re-testing on the post-#535 tree found zero
/// stalls across 80,000 rounds at both recorded contender counts. The bound is
/// deliberately kept anyway: the one-sided result says "no longer
/// reproducible", not "fixed", and the spurious `disconnected` error on the
/// same path still fails bursts, so a bound on a hang remains worth its cost.
///
/// A timeout turns a hang into a reported error. Without it, a benchmark run
/// and `cargo test --all-targets` (which executes `harness = false` benches in
/// Criterion's test mode) could block indefinitely. The defects are recorded in
/// `docs/runtime-ownership-baselines.md` and belong to ADR 013 / issue #647;
/// they are deliberately not fixed here, because issue #639 records baselines
/// rather than changing ownership topology.
pub const CONTENDED_BURST_TIMEOUT: Duration = Duration::from_secs(30);

/// The pool type every contended helper operates on.
type Pool = wireframe::client::WireframeClientPool<
    wireframe::serializer::BincodeSerializer,
    wireframe::test_helpers::pool_client::ClientHello,
    (),
>;

/// Run one contended burst, converting a stall into an error.
///
/// The timeout exists to bound a *hang*, not to measure speed: see
/// [`CONTENDED_BURST_TIMEOUT`]. Without it a deadlocked burst never returns and
/// the caller blocks forever.
pub(super) async fn contended_round_bounded(
    workload: PoolWorkload,
    pool: &Pool,
) -> Result<u64, String> {
    match tokio::time::timeout(CONTENDED_BURST_TIMEOUT, contended_round(workload, pool)).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "contended burst did not complete within {CONTENDED_BURST_TIMEOUT:?}: every contender \
             is stalled with waiters still queued (pool acquire-path hang, last observed before \
             #535 and not reproduced since; see docs/runtime-ownership-baselines.md and issue \
             #647)"
        )),
    }
}

/// Race every contender for pool capacity behind a shared start barrier.
///
/// The barrier matters: without it the first caller can complete every round
/// before the others are polled, which would make a "contended" label describe
/// an uncontended run. Contenders are given more rounds than the pool has slots
/// so that demand genuinely exceeds capacity once they are all running.
async fn contended_round(workload: PoolWorkload, pool: &Pool) -> Result<u64, String> {
    let contenders = workload.mode.contenders(workload.contenders);
    let barrier = Arc::new(Barrier::new(contenders));
    let granted = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut tasks = Vec::with_capacity(contenders);
    for _ in 0..contenders {
        let mut handle = pool.handle();
        let barrier = Arc::clone(&barrier);
        let granted = Arc::clone(&granted);
        let failures = Arc::clone(&failures);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            match acquire_round(&mut handle, CONTENDED_ROUNDS_PER_CONTENDER).await {
                Ok(count) => {
                    granted.fetch_add(
                        usize::try_from(count).unwrap_or(usize::MAX),
                        Ordering::SeqCst,
                    );
                }
                Err(err) => {
                    let mut guard = match failures.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.push(err.to_string());
                }
            }
        }));
    }

    for task in tasks {
        task.await
            .map_err(|err| format!("contended contender panicked: {err}"))?;
    }

    let errors = {
        let guard = match failures.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clone()
    };
    if let Some(first) = errors.first() {
        return Err(format!(
            "{} of {contenders} contenders failed; first: {first}",
            errors.len()
        ));
    }

    Ok(u64::try_from(granted.load(Ordering::SeqCst)).unwrap_or(u64::MAX))
}
