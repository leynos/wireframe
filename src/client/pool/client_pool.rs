//! Outer pooled-client type and slot-selection logic.

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use bb8::Pool;
use bincode::Encode;
use futures::{Future, future::select_all};

use super::{
    ClientPoolConfig,
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

type AcquirePermit<S, P, C> = (Arc<PoolSlot<S, P, C>>, tokio::sync::OwnedSemaphorePermit);

type AcquirePermitFuture<S, P, C> =
    Pin<Box<dyn Future<Output = Result<AcquirePermit<S, P, C>, ClientError>> + Send>>;

pub(crate) struct ClientPoolInner<S, P = (), C = ()>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) slots: Arc<[Arc<PoolSlot<S, P, C>>]>,
    pub(crate) next_slot: AtomicUsize,
    pub(crate) scheduler: Arc<PoolScheduler<S, P, C>>,
    shutdown: AtomicBool,
}

/// Pool of warm wireframe client sockets.
pub struct WireframeClientPool<S, P = (), C = ()>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    inner: Arc<ClientPoolInner<S, P, C>>,
}

impl<S, P, C> WireframeClientPool<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) async fn connect(
        addr: SocketAddr,
        pool_config: ClientPoolConfig,
        parts: ClientBuildParts<S, P, C>,
    ) -> Result<Self, ClientError> {
        let fairness_policy = pool_config.fairness_policy_value();
        let mut slots = Vec::with_capacity(pool_config.pool_size_value());
        for _ in 0..pool_config.pool_size_value() {
            let manager = WireframeConnectionManager::new(addr, parts.clone());
            let bb8_pool = Pool::builder()
                .max_size(1)
                .idle_timeout(Some(pool_config.idle_timeout_value()))
                .reaper_rate(pool_config.idle_timeout_value())
                .build(manager)
                .await?;
            slots.push(Arc::new(PoolSlot::new(
                bb8_pool,
                pool_config.max_in_flight_per_socket_value(),
                pool_config.idle_timeout_value(),
            )));
        }

        Ok(Self {
            inner: Arc::new(ClientPoolInner {
                slots: Arc::from(slots),
                next_slot: AtomicUsize::new(0),
                scheduler: Arc::new(PoolScheduler::new(fairness_policy)),
                shutdown: AtomicBool::new(false),
            }),
        })
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
        let handle_id = self.inner.scheduler.register_handle();
        PoolHandle::new(Arc::clone(&self.inner), handle_id)
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

    /// Close the pool and drop all warm sockets.
    pub async fn close(self) {
        self.inner.shutdown();
        tokio::task::yield_now().await;
        drop(self);
    }
}

impl<S, P, C> ClientPoolInner<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) fn is_shutdown(&self) -> bool { self.shutdown.load(Ordering::Acquire) }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.scheduler.notify_shutdown();
    }

    pub(crate) fn try_acquire_immediately(self: &Arc<Self>) -> Option<PooledClientLease<S, P, C>> {
        if self.is_shutdown() {
            return None;
        }
        self.ordered_slots().into_iter().find_map(|slot| {
            slot.try_acquire_permit()
                .map(|permit| PooledClientLease::new(slot, permit, Some(Arc::clone(self))))
        })
    }

    pub(crate) async fn acquire_slot_permit(&self) -> Result<AcquirePermit<S, P, C>, ClientError> {
        let waiters = self
            .ordered_slots()
            .into_iter()
            .map(|slot| {
                Box::pin(async move {
                    let permit = slot.acquire_permit().await?;
                    Ok::<_, ClientError>((slot, permit))
                }) as AcquirePermitFuture<S, P, C>
            })
            .collect::<Vec<_>>();
        let (result, ..) = select_all(waiters).await;
        result
    }

    fn ordered_slots(&self) -> Vec<Arc<PoolSlot<S, P, C>>> {
        let mut ordered = self.slots.iter().cloned().collect::<Vec<_>>();
        let len = ordered.len();
        if len > 0 {
            let start = self.next_slot.fetch_add(1, Ordering::Relaxed);
            ordered.rotate_left(start.wrapping_rem(len));
        }
        ordered
    }
}
