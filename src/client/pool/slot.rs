//! One logical pool slot: one physical socket plus admission permits.

use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use bb8::{Pool, PooledConnection};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

use super::{manager::WireframeConnectionManager, sync::lock_or_recover};
use crate::{client::ClientError, serializer::Serializer};

/// One physical socket slot backed by a `bb8` pool of size one.
pub(crate) struct PoolSlot<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) pool: Pool<WireframeConnectionManager<S, P, C>>,
    permits: Arc<Semaphore>,
    idle_timeout: Duration,
    last_returned_at: Mutex<Option<Instant>>,
}

impl<S, P, C> PoolSlot<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) fn new(
        pool: Pool<WireframeConnectionManager<S, P, C>>,
        max_in_flight_per_socket: usize,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            pool,
            permits: Arc::new(Semaphore::new(max_in_flight_per_socket)),
            idle_timeout,
            last_returned_at: Mutex::new(None),
        }
    }

    pub(crate) fn try_acquire_permit(self: &Arc<Self>) -> Option<OwnedSemaphorePermit> {
        self.permits.clone().try_acquire_owned().ok()
    }

    pub(crate) async fn acquire_permit(
        self: &Arc<Self>,
    ) -> Result<OwnedSemaphorePermit, ClientError> {
        self.permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ClientError::disconnected())
    }

    pub(crate) async fn checkout(&self) -> Result<SlotConnection<'_, S, P, C>, ClientError> {
        let mut connection = self.get_connection().await?;

        if self.should_recycle_idle() {
            connection.mark_broken();
            drop(connection);
            self.clear_last_returned_at();
            connection = self.get_connection().await?;
        }

        Ok(SlotConnection {
            connection,
            last_returned_at: &self.last_returned_at,
        })
    }

    async fn get_connection(
        &self,
    ) -> Result<PooledConnection<'_, WireframeConnectionManager<S, P, C>>, ClientError> {
        self.pool.get().await.map_err(|err| match err {
            bb8::RunError::User(error) => error,
            bb8::RunError::TimedOut => ClientError::from(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "pooled connection checkout timed out",
            )),
        })
    }

    fn should_recycle_idle(&self) -> bool {
        self.lock_last_returned_at()
            .as_ref()
            .is_some_and(|returned_at| returned_at.elapsed() >= self.idle_timeout)
    }

    fn clear_last_returned_at(&self) { *self.lock_last_returned_at() = None; }

    fn lock_last_returned_at(&self) -> MutexGuard<'_, Option<Instant>> {
        lock_or_recover(&self.last_returned_at)
    }
}

pub(crate) struct SlotConnection<'a, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    connection: PooledConnection<'a, WireframeConnectionManager<S, P, C>>,
    last_returned_at: &'a Mutex<Option<Instant>>,
}

impl<S, P, C> Deref for SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    type Target = super::managed::ManagedClientConnection<S, C>;

    fn deref(&self) -> &Self::Target { &self.connection }
}

impl<S, P, C> DerefMut for SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.connection }
}

impl<S, P, C> Drop for SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) {
        let mut last_returned_at = lock_or_recover(self.last_returned_at);

        if self.connection.is_broken() {
            *last_returned_at = None;
        } else {
            *last_returned_at = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for slot admission permits and idle-recycle bookkeeping.

    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use super::PoolSlot;
    use crate::{
        client::{
            ClientCodecConfig,
            SocketOptions,
            connect_parts::ClientBuildParts,
            hooks::{LifecycleHooks, RequestHooks},
            pool::manager::WireframeConnectionManager,
            tracing_config::TracingConfig,
        },
        serializer::BincodeSerializer,
    };

    /// Build a slot without connecting: `build_unchecked` creates the bb8
    /// pool lazily, and these tests never touch the socket path.
    fn test_slot(
        max_in_flight: usize,
        idle_timeout: Duration,
    ) -> Arc<PoolSlot<BincodeSerializer, (), ()>> {
        let addr = SocketAddr::from(([127, 0, 0, 1], 1));
        let parts = ClientBuildParts {
            serializer: BincodeSerializer,
            codec_config: ClientCodecConfig::default(),
            socket_options: SocketOptions::default(),
            preamble_config: None,
            lifecycle_hooks: LifecycleHooks::default(),
            request_hooks: RequestHooks::default(),
            tracing_config: TracingConfig::default(),
        };
        let pool = bb8::Pool::builder()
            .max_size(1)
            .build_unchecked(WireframeConnectionManager::new(addr, parts));
        Arc::new(PoolSlot::new(pool, max_in_flight, idle_timeout))
    }

    /// The synchronous fast path must hand out a permit while capacity
    /// remains and refuse once it is exhausted; forcing `None` would push
    /// every acquire onto the waiting path.
    #[tokio::test]
    async fn try_acquire_permit_reflects_capacity() {
        let slot = test_slot(1, Duration::from_secs(30));

        let held = slot.try_acquire_permit();
        assert!(held.is_some(), "fresh slot must yield an immediate permit");
        assert!(
            slot.try_acquire_permit().is_none(),
            "exhausted slot must refuse an immediate permit"
        );

        drop(held);
        assert!(
            slot.try_acquire_permit().is_some(),
            "released capacity must be immediately reusable"
        );
    }

    /// Clearing the idle timestamp must reset the recycle decision; leaving
    /// a stale timestamp would recycle the freshly created connection on the
    /// next checkout after a recycle whose lease never sets a new timestamp
    /// (for example when the replacement checkout fails).
    #[tokio::test]
    async fn clear_last_returned_at_resets_recycle_decision() {
        let slot = test_slot(1, Duration::from_millis(1));

        *slot.lock_last_returned_at() = Some(tokio::time::Instant::now() - Duration::from_mins(1));
        assert!(
            slot.should_recycle_idle(),
            "stale timestamp must trigger a recycle"
        );

        slot.clear_last_returned_at();
        assert!(
            slot.lock_last_returned_at().is_none(),
            "clear must remove the timestamp"
        );
        assert!(
            !slot.should_recycle_idle(),
            "cleared timestamp must not trigger a recycle"
        );
    }
}
