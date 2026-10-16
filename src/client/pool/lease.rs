//! Lease API for pooled wireframe clients.
//!
//! A lease holds one admission permit against a physical socket. Each
//! operation checks out the underlying warm socket from that slot, executes
//! the requested client method, then returns the socket to `bb8`.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::{
    net::TcpStream,
    sync::{Mutex, OwnedSemaphorePermit},
};

use super::{
    client_pool::ClientPoolInner,
    managed::ManagedClientConnection,
    slot::{PoolSlot, SlotConnection},
};
use crate::{
    app::Packet,
    client::{ClientError, WireframeClient},
    message::{DecodeWith, EncodeWith},
    rewind_stream::RewindStream,
    serializer::Serializer,
};

/// How a completed lease operation changes its split-phase response state.
#[derive(Clone, Copy)]
enum PendingResponseChange {
    /// A split-phase send may leave one response on the socket.
    Add,
    /// A split-phase receive consumed one response.
    Remove,
    /// A whole request-response operation manages its own response.
    None,
}

/// One acquired pooled-client lease.
pub struct PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Slot whose physical connection is checked out for each operation.
    slot: Arc<PoolSlot<S, P, C>>,
    /// Admission token held for the lifetime of this lease.
    _permit: OwnedSemaphorePermit,
    /// Scheduler state to notify when this lease releases capacity.
    release_inner: Option<Arc<ClientPoolInner<S, P, C>>>,
    /// Number of split-phase responses this lease has not consumed yet.
    pending_responses: AtomicUsize,
    /// Slot permit retained until this lease consumes its pending responses.
    pending_response_permit: Mutex<Option<OwnedSemaphorePermit>>,
}

impl<S, P, C> PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Construct a lease while retaining the permit until drop.
    pub(crate) fn new(
        slot: Arc<PoolSlot<S, P, C>>,
        permit: OwnedSemaphorePermit,
        release_inner: Option<Arc<ClientPoolInner<S, P, C>>>,
    ) -> Self {
        Self {
            slot,
            _permit: permit,
            release_inner,
            pending_responses: AtomicUsize::new(0),
            pending_response_permit: Mutex::new(None),
        }
    }

    /// Classify transport errors that make a pooled connection unsafe to reuse.
    const fn should_recycle(err: &ClientError) -> bool { err.should_recycle_connection() }

    /// Record and schedule a discard for a socket with an unread response.
    fn discard_connection_with_pending_response(&self) {
        self.slot.discard_next_checkout();
        self.pending_responses.store(0, Ordering::Release);
        crate::metrics::inc_pool_pending_response_discard();
        tracing::debug!(
            action = "discard_connection",
            pool_operation = "split_phase_exchange",
            reason = "response_pending",
            "discarding pooled client connection with an unread response"
        );
    }

    /// Reserve the gate while keeping an existing hold on the lease through checkout.
    async fn acquire_protocol_permit(
        &self,
        change: PendingResponseChange,
        retained_permit: &mut Option<OwnedSemaphorePermit>,
    ) -> Result<Option<OwnedSemaphorePermit>, ClientError> {
        if self.pending_responses.load(Ordering::Acquire) > 0
            && matches!(change, PendingResponseChange::None)
        {
            self.discard_connection_with_pending_response();
            drop(retained_permit.take());
            return Err(ClientError::disconnected());
        }

        if self.pending_responses.load(Ordering::Acquire) > 0 && retained_permit.is_none() {
            self.discard_connection_with_pending_response();
            return Err(ClientError::disconnected());
        }

        if retained_permit.is_some() {
            Ok(None)
        } else {
            self.slot.acquire_protocol_permit().await.map(Some)
        }
    }

    /// Check out a socket and invalidate pending split-phase state on failure.
    async fn checkout_connection(&self) -> Result<SlotConnection<'_, S, P, C>, ClientError> {
        match self.slot.checkout(&self.pending_responses).await {
            Ok(connection) => Ok(connection),
            Err(error) => {
                if self.pending_responses.load(Ordering::Acquire) > 0 {
                    self.discard_connection_with_pending_response();
                }
                Err(error)
            }
        }
    }

    /// Update pending-response state and decide whether the gate must stay held.
    fn finish_operation<R>(
        &self,
        connection: &mut SlotConnection<'_, S, P, C>,
        result: &Result<R, ClientError>,
        change: PendingResponseChange,
    ) -> bool {
        if let Err(err) = result
            && Self::should_recycle(err)
        {
            connection.mark_broken();
            self.pending_responses.store(0, Ordering::Release);
            false
        } else {
            if result.is_ok() {
                self.record_pending_response_change(change);
            }
            self.pending_responses.load(Ordering::Acquire) > 0
        }
    }

    /// Run one operation against a checked-out connection and mark bad sockets.
    async fn dispatch_on_connection<R>(
        &self,
        pending_response_change: PendingResponseChange,
        operation: impl AsyncFnOnce(&mut ManagedClientConnection<S, C>) -> Result<R, ClientError>,
    ) -> Result<R, ClientError> {
        let mut pending_response_permit = self.pending_response_permit.lock().await;
        let mut newly_acquired_permit = self
            .acquire_protocol_permit(pending_response_change, &mut pending_response_permit)
            .await?;
        let had_pending_response = self.pending_responses.load(Ordering::Acquire) > 0;
        let mut connection = match self.checkout_connection().await {
            Ok(connection) => connection,
            Err(error) => {
                if had_pending_response {
                    drop(pending_response_permit.take());
                }
                return Err(error);
            }
        };
        let result = operation(&mut connection).await;
        connection.disarm();
        let keep_protocol_permit =
            self.finish_operation(&mut connection, &result, pending_response_change);
        drop(connection);

        if keep_protocol_permit {
            if let Some(protocol_permit) = newly_acquired_permit.take() {
                *pending_response_permit = Some(protocol_permit);
            }
        } else {
            drop(newly_acquired_permit.take());
            drop(pending_response_permit.take());
        }
        result
    }

    /// Run an operation on the live client behind the pooled-connection guard.
    async fn dispatch_on_client<R>(
        &self,
        pending_response_change: PendingResponseChange,
        operation: impl AsyncFnOnce(
            &mut WireframeClient<S, RewindStream<TcpStream>, C>,
        ) -> Result<R, ClientError>,
    ) -> Result<R, ClientError> {
        self.dispatch_on_connection(pending_response_change, async |connection| {
            operation(connection.client_mut()?).await
        })
        .await
    }

    /// Record whether a split-phase exchange still has an unread response.
    fn record_pending_response_change(&self, change: PendingResponseChange) {
        match change {
            PendingResponseChange::Add => {
                self.pending_responses.fetch_add(1, Ordering::AcqRel);
            }
            PendingResponseChange::Remove => {
                let _ = self.pending_responses.fetch_update(
                    Ordering::AcqRel,
                    Ordering::Acquire,
                    |pending| Some(pending.saturating_sub(1)),
                );
            }
            PendingResponseChange::None => {}
        }
    }

    /// Send one message over the leased socket.
    ///
    /// A successful split-phase send leaves one response pending until a
    /// receive consumes it. Dropping the lease first discards its socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, or transport I/O
    /// fails.
    pub async fn send<M: EncodeWith<S>>(&self, message: &M) -> Result<(), ClientError> {
        self.dispatch_on_connection(PendingResponseChange::Add, async |conn| {
            conn.client_mut()?.send(message).await
        })
        .await
    }

    /// Receive one message from the leased socket.
    ///
    /// A successful receive consumes one response left pending by a split-phase
    /// send on this lease.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, decode, or transport I/O fails.
    pub async fn receive<M: DecodeWith<S>>(&self) -> Result<M, ClientError> {
        self.dispatch_on_connection(PendingResponseChange::Remove, async |conn| {
            conn.client_mut()?.receive().await
        })
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
        self.dispatch_on_client(PendingResponseChange::None, async |client| {
            client.call(request).await
        })
        .await
    }

    /// Send one envelope and return the correlation ID used.
    ///
    /// A successful split-phase send leaves one response pending until a
    /// receive consumes it. Dropping the lease first discards its socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, or transport I/O
    /// fails.
    pub async fn send_envelope<M>(&self, envelope: M) -> Result<u64, ClientError>
    where
        M: Packet + EncodeWith<S>,
    {
        self.dispatch_on_connection(PendingResponseChange::Add, async |conn| {
            conn.client_mut()?.send_envelope(envelope).await
        })
        .await
    }

    /// Receive one envelope from the leased socket.
    ///
    /// A successful receive consumes one response left pending by a split-phase
    /// send on this lease.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, decode, or transport I/O fails.
    pub async fn receive_envelope<M>(&self) -> Result<M, ClientError>
    where
        M: Packet + DecodeWith<S>,
    {
        self.dispatch_on_client(PendingResponseChange::Remove, async |client| {
            client.receive_envelope().await
        })
        .await
    }

    /// Send one envelope and await its correlated response.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, decode,
    /// correlation validation, or transport I/O fails.
    // @codescene(disable:"Code Duplication") The shared dispatcher preserves
    // distinct correlated-call semantics (2026-10-16).
    pub async fn call_correlated<M>(&self, request: M) -> Result<M, ClientError>
    where
        M: Packet + EncodeWith<S> + DecodeWith<S>,
    {
        self.dispatch_on_client(PendingResponseChange::None, async |client| {
            client.call_correlated(request).await
        })
        .await
    }
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;

impl<S, P, C> Drop for PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) {
        if self.pending_responses.load(Ordering::Acquire) > 0 {
            self.discard_connection_with_pending_response();
        }

        if let Some(inner) = &self.release_inner {
            inner.scheduler.notify_capacity_available(Arc::clone(inner));
        }
    }
}
