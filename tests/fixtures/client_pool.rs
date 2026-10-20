//! `ClientPoolWorld` fixture for rstest-bdd pooled-client scenarios.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::fixture;
use tokio::time::timeout;
use wireframe::{
    client::{ClientError, ClientPoolConfig},
    test_helpers::{
        Ping,
        Pong,
        PoolServerBehavior,
        PoolTestServer,
        TestClientPool,
        build_pooled_client,
    },
};
pub use wireframe_testing::TestResult;

/// Bound on the cancellation scenario's waits and teardown.
const CANCELLATION_GRACE: Duration = Duration::from_secs(1);
/// Physical slots used by the multi-slot rotation scenario.
const SLOT_COUNT: usize = 2;
/// Bound on waiting for an acquisition that must not be granted.
const SATURATION_GRACE: Duration = Duration::from_millis(25);
/// Bound on waiting for capacity released by a dropped lease.
const RECOVERY_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CancelledCallCapacity {
    /// The post-cancellation acquire has not been attempted.
    #[default]
    NotChecked,
    /// A fresh lease was granted after the cancellation completed.
    Released,
}

#[derive(Default)]
struct InFlightLimitOutcome {
    blocked_third_acquire: bool,
    recovered_after_release: bool,
}

/// Observed behaviour of one multi-slot rotation run.
#[derive(Default)]
struct MultiSlotRotationOutcome {
    /// Slots reported by the pool's public size accessor.
    slot_count: usize,
    /// Slot that granted the first lease.
    first_slot: usize,
    /// Slot that granted the second lease.
    second_slot: usize,
    /// Whether the two leases opened separate physical sockets.
    used_distinct_sockets: bool,
    /// Whether a third acquisition blocked while both slots were leased.
    blocked_when_saturated: bool,
    /// Whether dropping a lease returned capacity to a new acquisition.
    recovered_after_release: bool,
}

pub struct ClientPoolWorld {
    pool: Option<Arc<TestClientPool>>,
    server: Option<PoolTestServer>,
    preamble_callback_count: Arc<AtomicUsize>,
    in_flight_limit: InFlightLimitOutcome,
    blocked_waiter_within_limit: bool,
    rejected_waiter_above_limit: bool,
    reconnected_after_idle: bool,
    cancelled_call_capacity: CancelledCallCapacity,
    multi_slot_rotation: MultiSlotRotationOutcome,
}

impl Default for ClientPoolWorld {
    fn default() -> Self {
        Self {
            pool: None,
            server: None,
            preamble_callback_count: Arc::new(AtomicUsize::new(0)),
            in_flight_limit: InFlightLimitOutcome::default(),
            blocked_waiter_within_limit: false,
            rejected_waiter_above_limit: false,
            reconnected_after_idle: false,
            cancelled_call_capacity: CancelledCallCapacity::NotChecked,
            multi_slot_rotation: MultiSlotRotationOutcome::default(),
        }
    }
}

#[rustfmt::skip]
#[fixture]
pub fn client_pool_world() -> ClientPoolWorld {
    ClientPoolWorld::default()
}

impl ClientPoolWorld {
    async fn start_server(&mut self) -> TestResult {
        self.server = Some(PoolTestServer::start().await?);
        Ok(())
    }

    async fn connect_pool(&mut self, config: ClientPoolConfig) -> TestResult {
        let addr = self.server.as_ref().ok_or("server missing")?.addr;
        let pool = build_pooled_client(addr, config, self.preamble_callback_count.clone()).await?;
        self.pool = Some(Arc::new(pool));
        Ok(())
    }

    pub async fn run_warm_reuse_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(ClientPoolConfig::default().pool_size(1))
            .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;

        let lease = pool.acquire().await?;
        let first: Pong = lease.call(&Ping(7)).await?;
        if first != Pong(7) {
            return Err(format!("unexpected first response: {first:?}").into());
        }
        drop(lease);

        let lease = pool.acquire().await?;
        let second: Pong = lease.call(&Ping(8)).await?;
        if second != Pong(8) {
            return Err(format!("unexpected second response: {second:?}").into());
        }

        Ok(())
    }

    pub async fn run_in_flight_limit_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(
            ClientPoolConfig::default()
                .pool_size(1)
                .max_in_flight_per_socket(2),
        )
        .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;

        let first = pool.acquire().await?;
        let second = pool.acquire().await?;
        self.in_flight_limit.blocked_third_acquire =
            timeout(Duration::from_millis(25), pool.acquire())
                .await
                .is_err();

        drop(first);
        self.in_flight_limit.recovered_after_release = matches!(
            timeout(Duration::from_millis(100), pool.acquire()).await,
            Ok(Ok(_))
        );
        drop(second);
        Ok(())
    }

    pub async fn run_waiter_admission_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(ClientPoolConfig::default().pool_size(1).max_waiters(1))
            .await?;
        let pool = Arc::clone(self.pool.as_ref().ok_or("pool missing")?);
        let held_lease = pool.acquire().await?;
        let waiter_pool = Arc::clone(&pool);
        let mut waiter = tokio::spawn(async move { waiter_pool.acquire().await });

        self.blocked_waiter_within_limit = timeout(Duration::from_millis(25), &mut waiter)
            .await
            .is_err();
        self.rejected_waiter_above_limit = matches!(
            timeout(Duration::from_millis(100), pool.acquire()).await,
            Ok(Err(ClientError::PoolSaturated { limit: 1 }))
        );

        drop(held_lease);
        match waiter.await? {
            Ok(waiter_lease) => drop(waiter_lease),
            Err(ClientError::PoolSaturated { .. }) => {
                self.blocked_waiter_within_limit = false;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    pub async fn run_idle_recycle_scenario(&mut self) -> TestResult {
        tokio::time::pause();
        self.start_server().await?;
        let idle_timeout = Duration::from_millis(50);
        self.connect_pool(
            ClientPoolConfig::default()
                .pool_size(1)
                .idle_timeout(idle_timeout),
        )
        .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;

        let lease = pool.acquire().await?;
        let first: Pong = lease.call(&Ping(1)).await?;
        if first != Pong(1) {
            return Err(format!("unexpected first response: {first:?}").into());
        }
        drop(lease);

        tokio::time::advance(idle_timeout + idle_timeout).await;
        tokio::task::yield_now().await;

        let lease = pool.acquire().await?;
        let second: Pong = lease.call(&Ping(2)).await?;
        if second != Pong(2) {
            return Err(format!("unexpected second response: {second:?}").into());
        }
        self.reconnected_after_idle = true;
        Ok(())
    }

    /// Cancel a pooled call while its first response is still unwritten.
    ///
    /// The server only writes that late response after the client task is
    /// cancelled, so a reused socket would read a reply nobody asked for.
    pub async fn run_cancelled_call_scenario(&mut self) -> TestResult {
        self.server =
            Some(PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?);
        self.connect_pool(
            ClientPoolConfig::default()
                .pool_size(1)
                .max_in_flight_per_socket(1),
        )
        .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;
        let lease = pool.acquire().await?;
        let call_task = tokio::spawn(async move { lease.call::<Ping, Pong>(&Ping(1)).await });
        let server = self.server.as_ref().ok_or("server missing")?;
        timeout(CANCELLATION_GRACE, server.wait_for_delayed_first_response()).await?;

        call_task.abort();
        let Err(join_error) = timeout(CANCELLATION_GRACE, call_task).await? else {
            return Err("cancelled pooled call completed instead of stopping".into());
        };
        if !join_error.is_cancelled() {
            return Err("expected the pooled call task to be cancelled".into());
        }

        server.release_delayed_first_response();
        timeout(
            CANCELLATION_GRACE,
            server.wait_for_delayed_first_response_attempt(),
        )
        .await?;

        let lease = timeout(CANCELLATION_GRACE, pool.acquire()).await??;
        self.cancelled_call_capacity = CancelledCallCapacity::Released;
        let response: Pong = lease.call(&Ping(2)).await?;
        if response != Pong(2) {
            return Err(format!("unexpected response after cancellation: {response:?}").into());
        }

        Ok(())
    }

    pub fn warm_reuse_preserved(&self) -> bool {
        self.preamble_callback_count.load(Ordering::SeqCst) == 1
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 1 && server.connection_count() == 1
            })
    }

    pub fn blocked_third_acquire(&self) -> bool { self.in_flight_limit.blocked_third_acquire }

    pub fn recovered_after_release(&self) -> bool { self.in_flight_limit.recovered_after_release }

    pub fn bounded_waiter_admission_preserved(&self) -> bool {
        self.blocked_waiter_within_limit && self.rejected_waiter_above_limit
    }

    /// Check that cancellation released capacity and forced a fresh socket.
    pub fn reconnected_after_cancelled_call(&self) -> bool {
        self.cancelled_call_capacity == CancelledCallCapacity::Released
            && self.preamble_callback_count.load(Ordering::SeqCst) == 2
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 2 && server.connection_count() == 2
            })
    }

    pub fn reconnected_after_idle(&self) -> bool {
        self.reconnected_after_idle
            && self.preamble_callback_count.load(Ordering::SeqCst) == 2
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 2 && server.connection_count() == 2
            })
    }

    /// Acquire every slot once, then prove a saturated pool still rotates.
    ///
    /// With one admission permit per socket, two back-to-back acquisitions
    /// must land on different slots: the second attempt starts at the next
    /// rotated index and skips the slot the first lease still holds. A third
    /// acquisition then scans the whole rotation and finds no free capacity,
    /// so it waits instead of reusing a leased slot. Dropping a lease must
    /// return that waiting acquisition to service.
    pub async fn run_multi_slot_rotation_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(ClientPoolConfig::default().pool_size(SLOT_COUNT))
            .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;
        let outcome = &mut self.multi_slot_rotation;
        outcome.slot_count = pool.pool_size();

        let first = pool.acquire().await?;
        let second = pool.acquire().await?;
        outcome.first_slot = first.slot_id();
        outcome.second_slot = second.slot_id();

        // Each lease exercises its own physical socket, so the server must
        // observe one connection per configured slot.
        let _: Pong = first.call(&Ping(1)).await?;
        let _: Pong = second.call(&Ping(2)).await?;
        outcome.used_distinct_sockets = self.server.as_ref().is_some_and(|server| {
            server.connection_count() == SLOT_COUNT && server.preamble_count() == SLOT_COUNT
        });

        outcome.blocked_when_saturated = timeout(SATURATION_GRACE, pool.acquire()).await.is_err();

        drop(first);
        outcome.recovered_after_release =
            matches!(timeout(RECOVERY_GRACE, pool.acquire()).await, Ok(Ok(_)));
        Ok(())
    }

    /// Report the configured pool size observed through the public API.
    pub fn slot_count(&self) -> usize { self.multi_slot_rotation.slot_count }

    /// Check that rotation, saturation, and release all behaved as specified.
    pub fn multi_slot_rotation_preserved(&self) -> bool {
        let outcome = &self.multi_slot_rotation;
        outcome.slot_count == SLOT_COUNT
            && outcome.used_distinct_sockets
            && outcome.first_slot == 0
            && outcome.second_slot == 1
            && outcome.blocked_when_saturated
    }

    /// Check that dropping one lease let the saturated acquisition proceed.
    pub fn saturated_slot_recovered(&self) -> bool {
        self.multi_slot_rotation.recovered_after_release
    }
}
