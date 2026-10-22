//! `ClientPoolRotationWorld` fixture for pooled slot-rotation scenarios.
//!
//! These scenarios drive `WireframeClientPool::acquire` against a live test
//! server. They exist to prove, through the public API, that rotation really
//! moves between physical slots, that a saturated starting slot is skipped in
//! favour of a free one, and that a waiter parked while every slot is leased
//! is resumed by the same acquisition once capacity returns.

use std::{sync::Arc, time::Duration};

use rstest::fixture;
use tokio::time::timeout;
use wireframe::{
    client::{ClientPoolConfig, PoolTaskChurnObserver},
    test_helpers::{Ping, Pong, PoolTestServer, TestClientPool, build_preamble_pool},
};
pub use wireframe_testing::{TestError, TestResult};

/// Physical slots configured by every rotation scenario.
const SLOT_COUNT: usize = 2;
/// Bound on proving that an acquisition is still waiting for capacity.
const SATURATION_GRACE: Duration = Duration::from_millis(25);
/// Bound on acquiring capacity that a drop has already released.
const RECOVERY_GRACE: Duration = Duration::from_secs(1);
/// Acquire-then-release rounds used to observe the rotation cursor.
///
/// Four rounds over two slots must reach both slots twice. Four is even, so the
/// cursor is left on slot 0 for the concurrent phase that follows.
const ROTATION_ROUNDS: usize = 4;

/// Slots granted by successive acquire-then-release rounds.
///
/// Releasing between rounds is what makes the rotation observable: while a
/// lease is held, a stalled cursor would still produce alternating slots by
/// falling through the occupied one.
///
/// Neither the exact slot order nor the concurrent phase's starting slot is
/// asserted here. A scheduler worker left running by an earlier release can
/// claim a later acquisition and advance the same cursor from the permit-race
/// path, so how many advances a run performs depends on worker timing. That
/// arithmetic is pinned deterministically by
/// `client::pool::core::rotation_tests`. This layer asserts the pool-level
/// policy instead: every acquisition succeeds, the rounds reach both physical
/// slots, and the parked waiter resumes on the slot whose lease was dropped.
type RotationOrder = Vec<usize>;

/// Observed behaviour of the rotation-and-wait scenario.
#[derive(Default)]
struct MultiSlotRotationOutcome {
    /// Slots reported by the pool's public size accessor.
    slot_count: usize,
    /// Slot of the lease dropped to resume the parked acquisition.
    ///
    /// Recorded as an observation, not an expectation: the scheduler does not
    /// promise which slot the concurrent phase starts on.
    dropped_slot: usize,
    /// Slot of the lease retained across the parked acquisition.
    retained_slot: usize,
    /// Slot that granted capacity to the waiter after a lease was dropped.
    recovered_slot: Option<usize>,
    /// Slots granted by the acquire-then-release rounds.
    rotation_order: RotationOrder,
    /// Whether the two concurrent leases came from separate physical sockets.
    used_distinct_sockets: bool,
    /// Whether the pinned waiter was still pending while both slots were leased.
    waiter_stayed_pending: bool,
    /// Whether the resumed waiter's lease carried a request successfully.
    recovered_lease_usable: bool,
}

/// Observed behaviour of the saturated-start-skip scenario.
#[derive(Default)]
struct SaturatedStartOutcome {
    /// Slots reported by the pool's public size accessor.
    slot_count: usize,
    /// Slot kept leased while the reacquisition ran.
    held_slot: usize,
    /// Slot released before the reacquisition ran.
    released_slot: usize,
    /// Slot granted by the fast rotation walk, or `None` when it parked.
    ///
    /// The two facts share one field because they cannot be observed apart: a
    /// grant that parked was not granted by the walk. Keeping them together
    /// stops a later reader pairing a slot with the wrong path.
    fast_path_slot: Option<usize>,
    /// Whether the retained lease still carried a request afterwards.
    held_lease_still_usable: bool,
    /// Whether the reacquired lease carried a request successfully.
    reacquired_lease_usable: bool,
    /// Whether the two slots had connected as separate physical sockets.
    used_distinct_sockets: bool,
}

/// Observed result of leasing both slots and resuming a parked acquisition.
struct ConcurrentRotationOutcome {
    /// Slot of the lease dropped to resume the parked acquisition.
    dropped_slot: usize,
    /// Slot of the lease retained across the parked acquisition.
    retained_slot: usize,
    /// Whether the pinned waiter was still pending while both slots were leased.
    waiter_stayed_pending: bool,
    /// Slot that granted capacity to the waiter after a lease was dropped.
    recovered_slot: usize,
    /// Whether the resumed waiter's lease carried a request successfully.
    recovered_lease_usable: bool,
}

/// World backing the pooled slot-rotation scenarios.
#[derive(Default)]
pub struct ClientPoolRotationWorld {
    pool: Option<Arc<TestClientPool>>,
    server: Option<PoolTestServer>,
    rotation: MultiSlotRotationOutcome,
    saturated_start: SaturatedStartOutcome,
}

#[rustfmt::skip]
#[fixture]
pub fn client_pool_rotation_world() -> ClientPoolRotationWorld {
    ClientPoolRotationWorld::default()
}

impl ClientPoolRotationWorld {
    /// Start a live server and connect a two-slot pool with one permit each.
    ///
    /// The explicit per-socket permit limit is the precondition both rotation
    /// scenarios rely on: a leased slot has no spare capacity to hand out, so
    /// selection has to move on to a different slot.
    async fn start_rotation_pool(&mut self) -> TestResult {
        let config = ClientPoolConfig::default()
            .pool_size(SLOT_COUNT)
            .max_in_flight_per_socket(1);
        let (server, pool, _preamble_callback_count) = build_preamble_pool(config).await?;
        self.server = Some(server);
        self.pool = Some(Arc::new(pool));
        Ok(())
    }

    /// Borrow the pool that setup installed.
    fn pool(&self) -> Result<&Arc<TestClientPool>, &'static str> {
        self.pool.as_ref().ok_or("pool missing")
    }

    /// Prove rotation order, then that a parked acquisition resumes on release.
    ///
    /// The four acquire-then-release rounds must visit slots `0, 1, 0, 1`. The
    /// releases are what make the cursor observable: while a lease is held, a
    /// stalled cursor would still produce alternating slots by falling through
    /// the occupied one, so only a free pool distinguishes rotation from luck.
    ///
    /// The final phase leases both slots and parks a third acquisition.
    /// Releasing one lease is the only capacity change, so the resumed waiter
    /// must receive that released slot whatever the cursor's position.
    pub async fn run_multi_slot_rotation_scenario(&mut self) -> TestResult {
        self.start_rotation_pool().await?;
        let pool = Arc::clone(self.pool()?);
        self.rotation.slot_count = pool.pool_size();
        self.rotation.rotation_order = self.acquire_release_rounds(&pool).await?;
        let concurrent = Self::lease_both_and_resume_parked_waiter(&pool).await?;
        self.rotation.dropped_slot = concurrent.dropped_slot;
        self.rotation.retained_slot = concurrent.retained_slot;
        self.rotation.waiter_stayed_pending = concurrent.waiter_stayed_pending;
        self.rotation.recovered_slot = Some(concurrent.recovered_slot);
        self.rotation.recovered_lease_usable = concurrent.recovered_lease_usable;
        self.rotation.used_distinct_sockets = self.distinct_sockets_observed();
        Ok(())
    }

    /// Lease both slots, park a third acquisition, then resume it on release.
    ///
    /// The parked acquisition is pinned rather than raced with a timeout: the
    /// `&mut JoinHandle` wait observes that it stays pending without cancelling
    /// the task, so the later await is the same acquisition. Releasing the
    /// first lease is the only capacity change, which is what makes the
    /// recovered slot's identity a causal observation.
    ///
    /// Takes the shared pool by `Arc` because the spawned waiter must own one.
    async fn lease_both_and_resume_parked_waiter(
        pool: &Arc<TestClientPool>,
    ) -> Result<ConcurrentRotationOutcome, TestError> {
        let churn = pool.task_churn_observer();
        let first = pool.acquire().await?;
        let second = pool.acquire().await?;
        let first_pong: Pong = first.call(&Ping(1)).await?;
        let second_pong: Pong = second.call(&Ping(2)).await?;
        if first_pong != Pong(1) || second_pong != Pong(2) {
            return Err("a concurrent rotation lease returned an unexpected response".into());
        }

        let waiter_pool = Arc::clone(pool);
        let mut waiter = tokio::spawn(async move { waiter_pool.acquire().await });
        let dropped_slot = first.slot_id();
        let waiter_stayed_pending = wait_until_parked(&churn).await
            && timeout(SATURATION_GRACE, &mut waiter).await.is_err();

        drop(first);
        let recovered = timeout(RECOVERY_GRACE, waiter).await???;
        let recovered_pong: Pong = recovered.call(&Ping(3)).await?;
        Ok(ConcurrentRotationOutcome {
            dropped_slot,
            retained_slot: second.slot_id(),
            waiter_stayed_pending,
            recovered_slot: recovered.slot_id(),
            recovered_lease_usable: recovered_pong == Pong(3),
        })
    }

    /// Run three acquire-then-release rounds and record the granted slots.
    async fn acquire_release_rounds(&self, pool: &TestClientPool) -> Result<RotationOrder, String> {
        let mut order = Vec::with_capacity(ROTATION_ROUNDS);
        for round in 0..ROTATION_ROUNDS {
            let acquired = timeout(RECOVERY_GRACE, pool.acquire())
                .await
                .map_err(|_| format!("rotation round {round} timed out"))?;
            let lease =
                acquired.map_err(|error| format!("rotation round {round} failed: {error}"))?;
            order.push(lease.slot_id());
            let pong: Pong = lease
                .call(&Ping(1))
                .await
                .map_err(|error| format!("rotation round {round} failed: {error}"))?;
            if pong != Pong(1) {
                return Err(format!(
                    "rotation round {round} returned an unexpected response"
                ));
            }
            drop(lease);
        }
        Ok(order)
    }

    /// Release one of two leased slots and prove the next acquire skips the
    /// slot that is still held.
    ///
    /// With every other slot leased, a third acquisition can only succeed by
    /// walking past the occupied starting point to the free slot. The retained
    /// lease is then exercised to show the skip did not disturb its capacity.
    pub async fn run_saturated_start_skip_scenario(&mut self) -> TestResult {
        self.start_rotation_pool().await?;
        let pool = Arc::clone(self.pool()?);
        let churn = pool.task_churn_observer();
        self.saturated_start.slot_count = pool.pool_size();

        let held = pool.acquire().await?;
        let released = pool.acquire().await?;
        self.saturated_start.held_slot = held.slot_id();
        self.saturated_start.released_slot = released.slot_id();
        drop(released);

        let before = churn.snapshot();
        let reacquired = timeout(RECOVERY_GRACE, pool.acquire()).await??;
        // The immediate path is tried before the scheduler parks an
        // acquisition, so an unchanged wait counter proves the grant came from
        // the fast rotation walk rather than from the permit race. Without this
        // the slot id alone would also pass if rotation failed and the slow
        // path happened to pick the same free slot.
        let granted_without_waiting = churn.snapshot().since(before).capacity_waits == 0;
        self.saturated_start.fast_path_slot = granted_without_waiting.then(|| reacquired.slot_id());
        let reacquired_pong: Pong = reacquired.call(&Ping(3)).await?;
        self.saturated_start.reacquired_lease_usable = reacquired_pong == Pong(3);

        let held_pong: Pong = held.call(&Ping(4)).await?;
        self.saturated_start.held_lease_still_usable = held_pong == Pong(4);
        self.saturated_start.used_distinct_sockets = self.distinct_sockets_observed();
        Ok(())
    }

    /// Report whether the server saw one connection and preamble per slot.
    pub fn distinct_sockets_observed(&self) -> bool {
        self.server.as_ref().is_some_and(|server| {
            server.connection_count() == SLOT_COUNT && server.preamble_count() == SLOT_COUNT
        })
    }

    /// Report the configured pool size observed through the public API.
    pub fn slot_count(&self) -> usize { self.rotation.slot_count }

    /// Check that successive acquisitions rotated across both slots.
    ///
    /// A cursor pinned to one slot would hand that slot out every round, so
    /// requiring both slots to appear still fails that defect without depending
    /// on worker timing. The two concurrent leases must likewise be distinct:
    /// they are taken while no worker is pending, so a repeated slot there
    /// would mean selection ignored the leased slot rather than rotated.
    pub fn multi_slot_rotation_preserved(&self) -> bool {
        let outcome = &self.rotation;
        let mut visited = outcome.rotation_order.clone();
        visited.sort_unstable();
        visited.dedup();

        outcome.slot_count == SLOT_COUNT
            && outcome.rotation_order.len() == ROTATION_ROUNDS
            && visited == [0, 1]
            && outcome.dropped_slot != outcome.retained_slot
    }

    /// Check that the parked waiter resumed from the slot its lease released.
    pub fn saturated_slot_recovered(&self) -> bool {
        let outcome = &self.rotation;
        outcome.waiter_stayed_pending
            && outcome.recovered_slot == Some(outcome.dropped_slot)
            && outcome.recovered_lease_usable
            && outcome.used_distinct_sockets
    }

    /// Check that the reacquisition skipped the still-leased starting slot.
    ///
    /// A grant that the wait counter attributes to the fast path, and that
    /// landed on the released slot while its neighbour stayed leased, is the
    /// observable proof that the occupied slot was skipped mid-walk.
    pub fn saturated_start_skipped(&self) -> bool {
        let outcome = &self.saturated_start;
        outcome.slot_count == SLOT_COUNT
            && outcome.held_slot == 0
            && outcome.released_slot == 1
            && outcome.fast_path_slot == Some(outcome.released_slot)
            && outcome.reacquired_lease_usable
            && outcome.used_distinct_sockets
    }

    /// Check that the lease kept across the reacquisition still works.
    pub fn kept_lease_usable(&self) -> bool { self.saturated_start.held_lease_still_usable }
}

/// Wait until the pool records that an acquisition parked on capacity.
///
/// Polling the per-pool counter is what makes the pinning meaningful: without
/// it, an acquisition that had not yet reached the wait path would look
/// pending for reasons unrelated to saturation. The counter is only incremented
/// after the shutdown check and only when no slot had a free permit, so a
/// non-zero reading proves the waiter genuinely exhausted all rotation entries.
async fn wait_until_parked(churn: &PoolTaskChurnObserver) -> bool {
    let parked = async {
        loop {
            if churn.snapshot().capacity_waits > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    };
    timeout(RECOVERY_GRACE, parked).await.is_ok()
}
