//! Outer pooled-client type and slot-selection logic.

use std::{net::SocketAddr, sync::Arc};

use bb8::Pool;
use bincode::Encode;
use tokio::task::JoinHandle;

use super::{
    ClientPoolConfig,
    churn::{PoolTaskChurnCounts, PoolTaskChurnObserver},
    core::PoolCore,
    handle::PoolHandle,
    lease::PooledClientLease,
    manager::WireframeConnectionManager,
    scheduler::{self, SchedulerHandle},
    slot::PoolSlot,
};
use crate::{
    client::{ClientError, connect_parts::ClientBuildParts},
    serializer::Serializer,
};

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
    /// Cloneable command handle onto the pool's single scheduler task.
    scheduler: SchedulerHandle<S, P, C>,
    /// Join handle used by [`Self::close`] to await scheduler termination.
    task: JoinHandle<()>,
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

        let core = Arc::new(PoolCore::new(slots.into_boxed_slice()));
        let churn = Arc::new(PoolTaskChurnCounts::default());
        let (scheduler, task) = scheduler::start(&core, fairness_policy, max_waiters, churn);
        Ok(Self {
            core,
            scheduler,
            task,
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
    /// // The pool spawned its scheduler when it was built, so acquiring and
    /// // releasing a lease spawns nothing at all.
    /// assert_eq!(churn.scheduler_spawns, 0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn task_churn_observer(&self) -> PoolTaskChurnObserver {
        PoolTaskChurnObserver::new(self.scheduler.churn())
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
        let handle_id = self.scheduler.allocate_handle_id();
        PoolHandle::new(self.scheduler.clone(), handle_id)
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

    /// Close the pool to new acquisitions and await scheduler shutdown.
    ///
    /// Shutdown is published before this method returns: an outstanding handle
    /// that keeps the shared pool state alive still observes it, and any later
    /// acquisition fails with [`ClientError::PoolClosed`] instead of taking
    /// capacity mid-teardown. Every acquisition already queued is resolved with
    /// the same error rather than being left parked.
    ///
    /// The pool's single scheduler task is joined before this returns, so a
    /// caller can rely on the task being gone afterwards. Dropping the sockets
    /// is still deferred: a lease acquired before close is not revoked, and a
    /// `PoolHandle` or lease keeps the pool core (and therefore every slot and
    /// warm socket) alive until it is dropped.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};
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
    /// assert!(matches!(error, ClientError::PoolClosed));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(self) {
        let Self {
            core,
            scheduler,
            task,
        } = self;
        scheduler.mark_closed();
        // `PoolCore::shutdown` notifies the scheduler's biased shutdown arm,
        // which drains queued and buffered acquisitions before the task ends.
        // Closing the mailbox by dropping the last sender is the fallback
        // signal when every handle has already gone; dropping the core
        // reference here lets teardown proceed once the task releases its own.
        core.shutdown();
        drop(scheduler);
        drop(core);
        if let Err(error) = task.await {
            tracing::error!(%error, "client pool scheduler task terminated with a join error");
        }
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
    pub const fn slot_id(&self) -> usize { self.selected_slot_index() }
}

#[cfg(test)]
#[path = "client_pool_tests.rs"]
mod tests;
