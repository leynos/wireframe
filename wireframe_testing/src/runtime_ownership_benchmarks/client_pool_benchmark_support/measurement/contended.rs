//! The contended burst: a provably exhausted pool, then contenders racing for
//! the capacity it withholds.
//!
//! This regime exists to produce two things an uncontended run cannot: callers
//! that genuinely had to wait for capacity, and the scheduler churn that serving
//! them costs. Producing the first *reliably* is the hard part.
//!
//! An earlier revision did not produce it at all. Contenders raced behind a
//! start barrier and dropped each lease the moment they took it, so a burst of
//! `contenders × CONTENDED_ROUNDS_PER_CONTENDER` acquisitions could complete
//! without a single caller ever queuing — a start barrier makes overlap
//! *possible*, not actual, and a task that wins a permit can finish several
//! immediately-ready rounds before another is polled. Task-spawn counts were
//! then read as evidence that contention occurred, which they are not: they
//! witness the scheduler being entered, and the scheduler is entered on every
//! release.
//!
//! The regime is therefore restructured into phases. One handle first takes
//! every permit the pool can issue and holds them, which makes exhaustion a fact
//! rather than an inference: with zero free permits, every contender that asks
//! for one *must* park. The burst then waits for the pool's own counter to
//! report those parkings before releasing anything, and only afterwards runs the
//! rounds whose churn it wants to attribute to a contended pool.
//!
//! # Why waiting on a counter is not circular
//!
//! The wait is bounded by [`CONTENDED_WAIT_TIMEOUT`]. A pool whose counter never
//! moves does not pass the check by waiting forever; it fails, because the
//! timeout fires and the burst is reported as an error. The assertion the
//! harness makes afterwards — that the pool observed at least `contenders`
//! capacity waits — is therefore discriminating: it is false when the
//! instrumentation is broken. The complementary direction is covered by the
//! uncontended workloads, which assert the same counter is *zero* when the pool
//! is never exhausted, so a counter that merely increments wantonly fails there.

use std::{
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::Barrier;
use wireframe::client::{ClientError, PoolTaskChurnObserver};

use super::{
    acquire_round,
    phase::{TestHandle, TestPool},
    pool_config,
};
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

/// How long the burst waits for the pool to report the waits it forced.
///
/// Separate from [`CONTENDED_BURST_TIMEOUT`], and much shorter, because it
/// bounds a *verification* step rather than a hang: the contenders are already
/// parked by the time it starts, so the report either arrives promptly or the
/// exhaustion did not happen. A current-thread runtime crosses this loop's exit
/// condition in a handful of yields, so exceeding the bound is a real signal —
/// the pool did not exhaust, or its counter did not record it — and is worth
/// reporting rather than hanging on.
pub const CONTENDED_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// A lease type nameable without repeating its three type parameters.
type Lease = wireframe::client::PooledClientLease<
    wireframe::serializer::BincodeSerializer,
    wireframe::test_helpers::pool_client::ClientHello,
    (),
>;

/// How many held leases empty this workload's pool.
///
/// Every slot issues `max_in_flight_per_socket` permits, so holding that many
/// leases leaves no capacity to hand out. Derived from the configuration rather
/// than assumed to be one per slot, so the exhaustion stays exact if the
/// admission limit ever changes.
fn exhaustive_holdings(workload: PoolWorkload) -> usize {
    let config = pool_config(workload);
    config
        .pool_size_value()
        .saturating_mul(config.max_in_flight_per_socket_value())
}

/// Take `count` leases and hold every one of them.
///
/// They are returned in one vector so the pool stays empty until the caller
/// drops it: releasing any one of them early would hand capacity to a contender
/// and let that contender complete without ever waiting, which is precisely the
/// outcome the witness checks for.
async fn hold_all_capacity(handle: &mut TestHandle, count: usize) -> Result<Vec<Lease>, String> {
    let mut held = Vec::with_capacity(count);
    for _ in 0..count {
        let lease = handle
            .acquire()
            .await
            .map_err(|err| format!("could not take capacity to exhaust the pool: {err}"))?;
        held.push(lease);
    }
    Ok(held)
}

/// Run one contended burst, converting a stall into an error.
///
/// The timeout exists to bound a *hang*, not to measure speed: see
/// [`CONTENDED_BURST_TIMEOUT`]. Without it a deadlocked burst never returns and
/// the caller blocks forever.
pub(super) async fn contended_round_bounded(
    workload: PoolWorkload,
    pool: &TestPool,
    observer: &PoolTaskChurnObserver,
) -> Result<u64, String> {
    let burst = contended_round(workload, pool, observer);
    let Ok(result) = tokio::time::timeout(CONTENDED_BURST_TIMEOUT, burst).await else {
        return Err(format!(
            "contended burst did not complete within {CONTENDED_BURST_TIMEOUT:?}: every contender \
             is stalled with waiters still queued (pool acquire-path hang, last observed before \
             #535 and not reproduced since; see docs/runtime-ownership-baselines.md and issue \
             #647)"
        ));
    };
    result
}

/// Empty the pool, prove contenders had to wait for it, then run the rounds.
///
/// The leases taken to empty the pool are scaffolding, not measured work, and
/// are deliberately excluded from the returned grant count.
async fn contended_round(
    workload: PoolWorkload,
    pool: &TestPool,
    observer: &PoolTaskChurnObserver,
) -> Result<u64, String> {
    let contenders = workload.mode.contenders(workload.contenders);
    let mut holdings_handle = pool.handle();
    let held = hold_all_capacity(&mut holdings_handle, exhaustive_holdings(workload)).await?;
    let held_count = held.len();

    // Snapshot after the pool is empty, so the witness below counts only the
    // waits this burst forced rather than any an earlier phase left behind.
    let waits_before = observer.snapshot().capacity_waits;

    // Spawn the contenders, then wait for the pool to report them parking.
    // Nothing is released until that report arrives, which is what makes the
    // parked waiters an observed fact rather than a hopeful one.
    let blocked = contend(pool, contenders, 1);
    let reported = awaits_forced_waits(observer, waits_before, contenders).await;

    // Release the holdings as a unit, so every permit returns at once and the
    // parked contenders are served one at a time as each drops the lease it
    // just took. Done before propagating the witness error: a burst that could
    // not observe its waiters still has to give the pool its capacity back.
    drop(held);
    reported?;

    let blocked_grants = blocked.await?;
    if blocked_grants == 0 {
        return Err(format!(
            "a burst that held all {held_count} permits granted no contended lease"
        ));
    }

    let churn_grants = contend(pool, contenders, CONTENDED_ROUNDS_PER_CONTENDER).await?;
    Ok(blocked_grants.saturating_add(churn_grants))
}

/// Spawn one contender per caller, released together by a start barrier.
///
/// Each contender acquires and immediately drops `rounds_per_contender` leases.
/// The returned future is not awaited until the caller decides to release
/// capacity, which is what lets the blocked phase park its contenders first.
fn contend(
    pool: &TestPool,
    contenders: usize,
    rounds_per_contender: usize,
) -> impl std::future::Future<Output = Result<u64, String>> + '_ {
    let barrier = Arc::new(Barrier::new(contenders));
    let granted = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut tasks = Vec::with_capacity(contenders);
    for _ in 0..contenders {
        let mut handle = pool.handle();
        // Each contender needs its own `Arc` clone; the originals stay alive so
        // nothing is dropped while other contenders are still starting.
        let contender_barrier = Arc::clone(&barrier);
        let contender_granted = Arc::clone(&granted);
        let contender_failures = Arc::clone(&failures);
        tasks.push(tokio::spawn(async move {
            contender_barrier.wait().await;
            match acquire_round(&mut handle, rounds_per_contender).await {
                Ok(count) => {
                    contender_granted.fetch_add(
                        usize::try_from(count).unwrap_or(usize::MAX),
                        Ordering::SeqCst,
                    );
                }
                Err(err) => push_failure(&contender_failures, &err),
            }
        }));
    }

    async move {
        for task in tasks {
            task.await
                .map_err(|err| format!("contended contender panicked: {err}"))?;
        }
        collect_failures(&failures, contenders)?;
        Ok(u64::try_from(granted.load(Ordering::SeqCst)).unwrap_or(u64::MAX))
    }
}

/// Wait until the pool reports the waits the exhaustion forced.
///
/// Polls the counter with yields rather than blocking on a notification,
/// because nothing in the pool signals the moment a waiter parks. The poll is
/// bounded by [`CONTENDED_WAIT_TIMEOUT`], so a counter that never moves produces
/// an error instead of a hang — which is what keeps the harness's later
/// assertion on this counter meaningful.
async fn awaits_forced_waits(
    observer: &PoolTaskChurnObserver,
    waits_before: u64,
    contenders: usize,
) -> Result<u64, String> {
    let expected = u64::try_from(contenders).unwrap_or(u64::MAX);
    let target = waits_before.saturating_add(expected);

    let pending = async {
        loop {
            let waits = observer.snapshot().capacity_waits;
            if waits >= target {
                return waits;
            }
            tokio::task::yield_now().await;
        }
    };

    let Ok(waits) = tokio::time::timeout(CONTENDED_WAIT_TIMEOUT, pending).await else {
        return Err(format!(
            "pool was exhausted, so {contenders} contenders had to wait for capacity, but only {} \
             waits were recorded within {CONTENDED_WAIT_TIMEOUT:?}; the exhaustion or its \
             instrumentation is not working",
            observer
                .snapshot()
                .capacity_waits
                .saturating_sub(waits_before)
        ));
    };
    Ok(waits)
}

/// Record one contender's acquisition failure without losing the others.
fn push_failure(failures: &Mutex<Vec<String>>, error: &ClientError) {
    let mut guard = match failures.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.push(error.to_string());
}

/// Turn accumulated contender failures into one reportable error.
///
/// Every contender's failure is kept, not just the first, because the spurious
/// `disconnected` this path is known to produce arrives in ones and twos and the
/// count is the difference between a flake and a systematic break. The report
/// names the first message so the error text stays greppable.
fn collect_failures(failures: &Mutex<Vec<String>>, contenders: usize) -> Result<(), String> {
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
    Ok(())
}
