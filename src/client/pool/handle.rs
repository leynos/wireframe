//! Logical-session handle API for fair pooled acquisition.

use super::{lease::PooledClientLease, scheduler::SchedulerHandle};
use crate::{
    client::ClientError,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

/// Stable logical-session identity for fair pooled acquisition.
///
/// A `PoolHandle` lets one caller participate in the pool's fairness policy
/// over time. It is the preferred API when a logical session repeatedly
/// acquires pooled leases and you want the scheduler to account for that
/// identity explicitly.
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
/// let mut session: PoolHandle<_, _, _> = pool.handle();
/// let lease = session.acquire().await?;
/// drop(lease);
/// # Ok(())
/// # }
/// ```
pub struct PoolHandle<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Command handle onto the pool's scheduler task.
    ///
    /// Holding a clone keeps the scheduler alive, which is what lets a handle
    /// outlive [`WireframeClientPool`](super::WireframeClientPool) without
    /// leaving queued work unserviced.
    scheduler: SchedulerHandle<S, P, C>,
    /// Scheduler identity removed when this handle is dropped.
    handle_id: u64,
}

impl<S, P, C> PoolHandle<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Bind a scheduler-assigned identity to the pool's command handle.
    pub(crate) const fn new(scheduler: SchedulerHandle<S, P, C>, handle_id: u64) -> Self {
        Self {
            scheduler,
            handle_id,
        }
    }

    /// Acquire one pooled-client lease under the configured fairness policy.
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
    /// let mut session: PoolHandle<_, _, _> = pool.handle();
    /// let lease = session.acquire().await?;
    /// drop(lease);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if permit acquisition or pooled checkout fails,
    /// including when the pool has been closed.
    ///
    /// A closed pool reports [`ClientError::PoolClosed`]; a pool whose
    /// scheduler task failed reports [`ClientError::SchedulerFailed`].
    pub async fn acquire(&mut self) -> Result<PooledClientLease<S, P, C>, ClientError> {
        self.scheduler.acquire(self.handle_id).await
    }

    /// Perform a complete request-response round trip through a fair lease.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// use wireframe::client::{ClientPoolConfig, PoolHandle, WireframeClient};
    ///
    /// #[derive(bincode::Encode, bincode::Decode)]
    /// struct Ping(u8);
    ///
    /// #[derive(bincode::Encode, bincode::Decode)]
    /// struct Pong(u8);
    ///
    /// # async fn demo(addr: SocketAddr) -> Result<(), wireframe::client::ClientError> {
    /// let pool = WireframeClient::builder()
    ///     .connect_pool(addr, ClientPoolConfig::default())
    ///     .await?;
    /// let mut session: PoolHandle<_, _, _> = pool.handle();
    /// let _reply: Pong = session.call(&Ping(1)).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if lease acquisition, serialization, decode, or
    /// transport I/O fails.
    pub async fn call<Req, Resp>(&mut self, request: &Req) -> Result<Resp, ClientError>
    where
        Req: EncodeWith<S>,
        Resp: DecodeWith<S>,
    {
        let lease = self.acquire().await?;
        lease.call(request).await
    }
}

impl<S, P, C> Drop for PoolHandle<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) { self.scheduler.deregister(self.handle_id); }
}
