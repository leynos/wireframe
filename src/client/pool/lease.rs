//! Lease API for pooled wireframe clients.
//!
//! A lease holds one admission permit against a physical socket. Each
//! operation checks out the underlying warm socket from that slot, executes
//! the requested client method, then returns the socket to `bb8`.

use std::sync::Arc;

use tokio::sync::OwnedSemaphorePermit;

use super::{
    client_pool::PoolSchedulerControl,
    core::PoolCore,
    managed::ManagedClientConnection,
    slot::PoolSlot,
};
use crate::{
    app::Packet,
    client::ClientError,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

/// One acquired pooled-client lease.
pub struct PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Shared root keeping every physical slot alive for this lease.
    core: Arc<PoolCore<S, P, C>>,
    /// Internal identity of the selected physical slot.
    slot_index: usize,
    /// Admission token held for the lifetime of this lease.
    _permit: OwnedSemaphorePermit,
    /// Separate scheduler handle to notify when this lease releases capacity.
    release_control: Option<Arc<PoolSchedulerControl<S, P, C>>>,
}

impl<S, P, C> PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Construct a lease only when its index resolves in the shared core.
    pub(crate) fn new(
        core: Arc<PoolCore<S, P, C>>,
        slot_index: usize,
        permit: OwnedSemaphorePermit,
        release_control: Option<Arc<PoolSchedulerControl<S, P, C>>>,
    ) -> Option<Self> {
        core.slot(slot_index)?;
        Some(Self {
            core,
            slot_index,
            _permit: permit,
            release_control,
        })
    }

    /// Resolve the selected slot through the core's checked internal accessor.
    pub(crate) fn slot(&self) -> Option<&PoolSlot<S, P, C>> { self.core.slot(self.slot_index) }

    /// Classify transport errors that make a pooled connection unsafe to reuse.
    const fn should_recycle(err: &ClientError) -> bool { err.should_recycle_connection() }

    /// Run one operation against a checked-out connection and mark bad sockets.
    async fn dispatch_on_connection<R>(
        &self,
        operation: impl AsyncFnOnce(&mut ManagedClientConnection<S, C>) -> Result<R, ClientError>,
    ) -> Result<R, ClientError> {
        let slot = self.slot().ok_or_else(ClientError::disconnected)?;
        let mut connection = slot.checkout().await?;
        let result = operation(&mut connection).await;
        if let Err(err) = &result
            && Self::should_recycle(err)
        {
            connection.mark_broken();
        }
        result
    }

    /// Send one message over the leased socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, or transport I/O
    /// fails.
    pub async fn send<M: EncodeWith<S>>(&self, message: &M) -> Result<(), ClientError> {
        self.dispatch_on_connection(async |conn| conn.client_mut()?.send(message).await)
            .await
    }

    /// Receive one message from the leased socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, decode, or transport I/O fails.
    pub async fn receive<M: DecodeWith<S>>(&self) -> Result<M, ClientError> {
        self.dispatch_on_connection(async |conn| conn.client_mut()?.receive().await)
            .await
    }

    /// Send one request and await one response over the leased socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, decode, or
    /// transport I/O fails.
    pub async fn call<Req, Resp>(&self, request: &Req) -> Result<Resp, ClientError>
    where
        Req: EncodeWith<S>,
        Resp: DecodeWith<S>,
    {
        self.dispatch_on_connection(async |conn| conn.client_mut()?.call(request).await)
            .await
    }

    /// Send one envelope and return the correlation ID used.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, or transport I/O
    /// fails.
    pub async fn send_envelope<M>(&self, envelope: M) -> Result<u64, ClientError>
    where
        M: Packet + EncodeWith<S>,
    {
        self.dispatch_on_connection(async |conn| conn.client_mut()?.send_envelope(envelope).await)
            .await
    }

    /// Receive one envelope from the leased socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, decode, or transport I/O fails.
    pub async fn receive_envelope<M>(&self) -> Result<M, ClientError>
    where
        M: Packet + DecodeWith<S>,
    {
        self.dispatch_on_connection(async |conn| conn.client_mut()?.receive_envelope().await)
            .await
    }

    /// Send one envelope and await its correlated response.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, decode,
    /// correlation validation, or transport I/O fails.
    pub async fn call_correlated<M>(&self, request: M) -> Result<M, ClientError>
    where
        M: Packet + EncodeWith<S> + DecodeWith<S>,
    {
        self.dispatch_on_connection(async |conn| conn.client_mut()?.call_correlated(request).await)
            .await
    }
}

impl<S, P, C> Drop for PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) {
        if let Some(control) = &self.release_control {
            control
                .scheduler
                .notify_capacity_available(Arc::clone(&self.core), Arc::clone(control));
        }
    }
}
