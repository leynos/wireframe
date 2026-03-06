//! One logical pool slot: one physical socket plus admission permits.

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};

use bb8::{Pool, PooledConnection};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

use super::manager::WireframeConnectionManager;
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
    last_returned_at: tokio::sync::Mutex<Option<Instant>>,
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
            last_returned_at: tokio::sync::Mutex::new(None),
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
        let should_recycle = self.should_recycle_idle().await;
        let mut connection = self.pool.get().await.map_err(|err| match err {
            bb8::RunError::User(error) => error,
            bb8::RunError::TimedOut => ClientError::from(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "pooled connection checkout timed out",
            )),
        })?;

        if should_recycle {
            connection.mark_broken();
            drop(connection);
            connection = self.pool.get().await.map_err(|err| match err {
                bb8::RunError::User(error) => error,
                bb8::RunError::TimedOut => ClientError::from(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "pooled connection checkout timed out",
                )),
            })?;
        }

        Ok(SlotConnection {
            connection,
            last_returned_at: &self.last_returned_at,
        })
    }

    async fn should_recycle_idle(&self) -> bool {
        let last_returned_at = self.last_returned_at.lock().await;
        last_returned_at
            .as_ref()
            .is_some_and(|returned_at| returned_at.elapsed() >= self.idle_timeout)
    }
}

pub(crate) struct SlotConnection<'a, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    connection: PooledConnection<'a, WireframeConnectionManager<S, P, C>>,
    last_returned_at: &'a tokio::sync::Mutex<Option<Instant>>,
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
        if let Ok(mut last_returned_at) = self.last_returned_at.try_lock() {
            if self.connection.is_broken() {
                *last_returned_at = None;
            } else {
                *last_returned_at = Some(Instant::now());
            }
        }
    }
}
