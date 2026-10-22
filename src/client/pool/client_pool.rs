//! Outer pooled-client type and slot-selection logic.

use std::{net::SocketAddr, sync::Arc};

use bb8::Pool;
use bincode::Encode;

use super::{
    ClientPoolConfig,
    churn::{PoolTaskChurnCounts, PoolTaskChurnObserver},
    core::PoolCore,
    handle::PoolHandle,
    lease::PooledClientLease,
    manager::WireframeConnectionManager,
    scheduler::PoolScheduler,
    slot::PoolSlot,
};
use crate::{
    client::{ClientError, connect_parts::ClientBuildParts},
    serializer::Serializer,
};

/// Cloneable scheduler state kept separate from physical slot ownership.
pub(crate) struct PoolSchedulerControl<S, P = (), C = ()>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Fairness coordinator for handles waiting on slot capacity.
    pub(crate) scheduler: Arc<PoolScheduler<S, P, C>>,
    /// Task-churn counters scoped to this pool rather than the process.
    ///
    /// Per-pool so a delta read across one workload cannot absorb churn from a
    /// pool another test is driving concurrently. See
    /// [`PoolTaskChurnObserver`].
    pub(crate) churn: Arc<PoolTaskChurnCounts>,
}

impl<S, P, C> PoolSchedulerControl<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Create scheduler bookkeeping independently from physical slot storage.
    pub(crate) fn new(fairness_policy: super::PoolFairnessPolicy, max_waiters: usize) -> Self {
        Self {
            scheduler: Arc::new(PoolScheduler::new(fairness_policy, max_waiters)),
            churn: Arc::new(PoolTaskChurnCounts::default()),
        }
    }

    /// Make an immediate lease while the core retains its selected slot.
    pub(crate) fn try_acquire_immediately(
        self: &Arc<Self>,
        core: &Arc<PoolCore<S, P, C>>,
    ) -> Option<PooledClientLease<S, P, C>> {
        let (slot_index, permit) = core.try_acquire_permit()?;
        PooledClientLease::new(Arc::clone(core), slot_index, permit, Some(Arc::clone(self)))
    }

    /// Close the scheduler side of the pool and drain queued acquisitions.
    ///
    /// Shutdown is split across the two roots that own the relevant state:
    /// `PoolCore` publishes the shutdown flag and wakes permit waits, while
    /// the scheduler must resolve the requests it has already queued. Draining
    /// them here means a `PoolHandle` that outlives [`WireframeClientPool`]
    /// cannot be left blocked on a pool that is going away.
    pub(crate) fn shutdown(&self) { self.scheduler.notify_shutdown(); }
}

/// Pool of configured slots that create physical wireframe connections on
/// demand.
pub struct WireframeClientPool<S, P = (), C = ()>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Shared fixed physical slots retained by every live lease.
    core: Arc<PoolCore<S, P, C>>,
    /// Fairness and release-notification state, without slot ownership.
    control: Arc<PoolSchedulerControl<S, P, C>>,
}

impl<S, P, C> WireframeClientPool<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Build the configured slots; physical connections are created on demand
    /// when a lease checks out a slot.
    pub(crate) async fn connect(
        addr: SocketAddr,
        pool_config: ClientPoolConfig,
        parts: ClientBuildParts<S, P, C>,
    ) -> Result<Self, ClientError> {
        let fairness_policy = pool_config.fairness_policy_value();
        let max_waiters = pool_config.max_waiters_value();
        let mut slots = Vec::with_capacity(pool_config.pool_size_value());
        for _ in 0..pool_config.pool_size_value() {
            let manager = WireframeConnectionManager::new(addr, parts.clone());
            let bb8_pool = Pool::builder()
                .max_size(1)
                .idle_timeout(Some(pool_config.idle_timeout_value()))
                .reaper_rate(pool_config.idle_timeout_value())
                .build(manager)
                .await?;
            slots.push(PoolSlot::new(
                bb8_pool,
                pool_config.max_in_flight_per_socket_value(),
                pool_config.idle_timeout_value(),
            ));
        }

        Ok(Self {
            core: Arc::new(PoolCore::new(slots.into_boxed_slice())),
            control: Arc::new(PoolSchedulerControl::new(fairness_policy, max_waiters)),
        })
    }

    /// Take a read-only handle onto this pool's task-churn counters.
    ///
    /// The counters are scoped to this pool, so a delta between two snapshots
    /// can only be attributed to work done on this pool. They are not shared
    /// process-wide: a benchmark that reads a delta while an unrelated test
    /// drives a different pool sees only its own pool's activity.
    ///
    /// Taking an observer is passive — it performs no acquire and spawns no
    /// task — so a bracket built from snapshots cannot perturb what it
    /// measures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientPoolConfig, WireframeClient};
    ///
    /// # async fn demo(addr: SocketAddr) -> Result<(), wireframe::client::ClientError> {
    /// let pool = WireframeClient::builder()
    ///     .connect_pool(addr, ClientPoolConfig::default())
    ///     .await?;
    /// let observer = pool.task_churn_observer();
    /// let before = observer.snapshot();
    ///
    /// let lease = pool.acquire().await?;
    /// drop(lease);
    ///
    /// let churn = observer.snapshot().since(before);
    /// // One acquire/release transition, so at most one worker: a worker an
    /// // earlier acquisition left servicing could make this zero.
    /// assert!(churn.scheduler_spawns <= 2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn task_churn_observer(&self) -> PoolTaskChurnObserver {
        PoolTaskChurnObserver::new(Arc::clone(&self.control.churn))
    }

    /// Create a stable logical-session handle governed by the pool scheduler.
    ///
    /// Prefer a `PoolHandle` when one logical session repeatedly acquires
    /// pooled leases and should receive fair turns relative to other blocked
    /// sessions.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientPoolConfig, PoolHandle, WireframeClient};
    ///
    /// # async fn demo(addr: SocketAddr) -> Result<(), wireframe::client::ClientError> {
    /// let pool = WireframeClient::builder()
    ///     .connect_pool(addr, ClientPoolConfig::default())
    ///     .await?;
    /// let mut handle: PoolHandle<_, _, _> = pool.handle();
    /// let lease = handle.acquire().await?;
    /// drop(lease);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn handle(&self) -> PoolHandle<S, P, C> {
        let handle_id = self.control.scheduler.register_handle();
        PoolHandle::new(Arc::clone(&self.core), Arc::clone(&self.control), handle_id)
    }

    /// Acquire one pooled-client lease.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if permit acquisition or pooled checkout fails.
    pub async fn acquire(&self) -> Result<PooledClientLease<S, P, C>, ClientError> {
        let mut handle = self.handle();
        handle.acquire().await
    }

    /// Report how many physical slots this pool owns.
    ///
    /// The count is fixed at construction; it is exposed through the
    /// `test-support` feature so behavioural tests can assert that they really
    /// exercised a multi-slot pool.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientPoolConfig, WireframeClient};
    ///
    /// # async fn demo(addr: SocketAddr) -> Result<(), wireframe::client::ClientError> {
    /// let pool = WireframeClient::builder()
    ///     .connect_pool(addr, ClientPoolConfig::default().pool_size(3))
    ///     .await?;
    /// assert_eq!(pool.pool_size(), 3);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn pool_size(&self) -> usize { self.core.pool_size() }

    /// Close the pool to new acquisitions and begin teardown.
    ///
    /// Shutdown is published before this method returns: an outstanding handle
    /// that keeps the shared pool state alive still observes it, and any later
    /// acquisition fails with a disconnected error instead of taking capacity
    /// mid-teardown.
    ///
    /// Dropping the sockets is deferred. A lease acquired before close is not
    /// revoked, and a `PoolHandle` or lease keeps the pool core (and therefore
    /// every slot and warm socket) alive until it is dropped.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientPoolConfig, WireframeClient};
    ///
    /// # async fn demo(addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    /// let pool = WireframeClient::builder()
    ///     .connect_pool(addr, ClientPoolConfig::default())
    ///     .await?;
    /// let mut handle = pool.handle();
    /// pool.close().await;
    ///
    /// let error = match handle.acquire().await {
    ///     Ok(_lease) => panic!("closed pool must refuse"),
    ///     Err(error) => error,
    /// };
    /// assert!(error.should_recycle_connection());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(self) {
        self.core.shutdown();
        self.control.shutdown();
        tokio::task::yield_now().await;
        drop(self);
    }
}

impl<S, P, C> PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Report which physical slot this lease was granted from.
    ///
    /// The identifier is a stable index within this lease's pool, not a socket
    /// address or connection identifier. It is exposed through the
    /// `test-support` feature so behavioural tests can assert rotation order
    /// through the public acquisition API.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientPoolConfig, PooledClientLease, WireframeClient};
    ///
    /// # async fn demo(addr: SocketAddr) -> Result<(), wireframe::client::ClientError> {
    /// let pool = WireframeClient::builder()
    ///     .connect_pool(addr, ClientPoolConfig::default().pool_size(2))
    ///     .await?;
    /// let first = pool.acquire().await?;
    /// let second = pool.acquire().await?;
    /// assert_ne!(first.slot_id(), second.slot_id());
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn slot_id(&self) -> usize { self.selected_slot_index() }
}

#[cfg(test)]
#[path = "client_pool_tests.rs"]
mod tests;
