//! One bracketed acquisition phase: build, acquire, settle, close.
//!
//! The bracket is the point of this module. Pool construction and teardown
//! happen *outside* the measured region, and that placement was forced by
//! measurement rather than chosen for tidiness.

use wireframe::client::pool_scheduler_task_spawns;

use super::{acquire_round, contended::contended_round_bounded, pool_config, settled_alive_tasks};
use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::workload::PoolWorkload;

/// Grant count and task-churn counters for one acquisition phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PhaseObservation {
    /// Leases granted during the phase.
    pub(super) granted: u64,
    /// Scheduler worker tasks spawned during the phase.
    pub(super) spawns: u64,
    /// Live tasks immediately before the phase.
    pub(super) alive_before: usize,
    /// Live tasks immediately after the phase.
    pub(super) alive_after: usize,
}

/// Read the settled gauges and package them for a completed acquisition.
///
/// A worker spawned by the final release observes an empty queue and exits on
/// its first poll. Wait for that to happen before reading the gauge, so
/// `alive_after` reflects settled state rather than a task already on its way
/// out. See [`settled_alive_tasks`] for why one yield is not enough.
///
/// This runs only once the acquisition itself has succeeded. After a stall the
/// gauge is not expected to hold steady, and [`settled_alive_tasks`] would fail
/// for a reason that only restates the stall; the caller discards the reading
/// with the error instead.
async fn settled_observation(
    granted: u64,
    spawns_before: u64,
    alive_before: usize,
) -> Result<PhaseObservation, String> {
    let alive_after = settled_alive_tasks().await?;
    let spawns = pool_scheduler_task_spawns().saturating_sub(spawns_before);

    Ok(PhaseObservation {
        granted,
        spawns,
        alive_before,
        alive_after,
    })
}

/// Run one iteration, bracketing only the acquisition phase.
///
/// Pool construction and teardown happen *outside* the bracketed region. That
/// placement is deliberate and was forced by measurement: `build_preamble_pool`
/// connects eagerly, so at pool sizes 1, 4, and 16 the construction phase alone
/// leaves exactly that many extra tasks alive. Sampling the task gauge across
/// construction would therefore report `pool_size` as though it were
/// acquisition churn, which is the opposite of what this scope measures.
///
/// The pool is closed on the error path too, and the acquisition error is
/// carried past the close rather than returned through it. A contended burst
/// that times out has already been detached: the timeout drops `contended_round`
/// and with it that function's `JoinHandle`s, which detaches the contender tasks
/// instead of aborting them. Those tasks hold `PoolHandle` clones, so dropping
/// the pool would leave its inner alive and nothing would ever shut it down —
/// the stalled contenders would keep racing against the next iteration's budget
/// and against the retries in the callers. Shutting down here wakes the queued
/// waiters with `disconnected` and ends them. It cannot block: `shutdown` is
/// synchronous, and the detached handles keep the inner alive, so the drop that
/// follows it is an `Arc` decrement rather than a teardown.
pub(super) async fn pool_once(workload: PoolWorkload) -> Result<PhaseObservation, String> {
    let (_server, pool, _preamble) =
        wireframe::test_helpers::pool_client::build_preamble_pool(pool_config(workload))
            .await
            .map_err(|err| format!("failed to build pooled client: {err}"))?;

    let spawns_before = pool_scheduler_task_spawns();
    let alive_before = super::alive_tasks();

    let acquisition = if workload.mode.is_contended() {
        contended_round_bounded(workload, &pool).await
    } else {
        let mut handle = pool.handle();
        acquire_round(&mut handle, workload.acquisitions)
            .await
            .map_err(|err| format!("uncontended acquire failed: {err}"))
    };

    let observed = match acquisition {
        Ok(granted) => settled_observation(granted, spawns_before, alive_before).await,
        Err(error) => Err(error),
    };

    pool.close().await;

    observed
}
