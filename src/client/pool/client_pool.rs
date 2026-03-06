//! Outer pooled-client type and slot-selection logic.

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bb8::Pool;
use bincode::Encode;
use futures::{Future, future::select_all};

use super::{
    ClientPoolConfig,
    lease::PooledClientLease,
    manager::WireframeConnectionManager,
    slot::PoolSlot,
};
use crate::{
    client::{ClientError, connect_parts::ClientBuildParts},
    serializer::Serializer,
};

type AcquirePermit<S, P, C> = (
    Arc<PoolSlot<S, P, C>>,
    tokio::sync::OwnedSemaphorePermit,
);

type AcquirePermitFuture<S, P, C> =
    Pin<Box<dyn Future<Output = Result<AcquirePermit<S, P, C>, ClientError>> + Send>>;

/// Pool of warm wireframe client sockets.
pub struct WireframeClientPool<S, P = (), C = ()>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    slots: Arc<[Arc<PoolSlot<S, P, C>>]>,
    next_slot: AtomicUsize,
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
            slots: Arc::from(slots),
            next_slot: AtomicUsize::new(0),
        })
    }

    /// Acquire one pooled-client lease.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if permit acquisition or pooled checkout fails.
    pub async fn acquire(&self) -> Result<PooledClientLease<S, P, C>, ClientError> {
        if let Some((slot, permit)) = self.try_acquire_immediately() {
            return Ok(PooledClientLease::new(slot, permit));
        }

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
        let (slot, permit) = result?;
        Ok(PooledClientLease::new(slot, permit))
    }

    fn try_acquire_immediately(&self) -> Option<AcquirePermit<S, P, C>> {
        self.ordered_slots()
            .into_iter()
            .find_map(|slot| slot.try_acquire_permit().map(|permit| (slot, permit)))
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

    /// Close the pool and drop all warm sockets.
    pub async fn close(self) {
        tokio::task::yield_now().await;
        drop(self);
    }
}
