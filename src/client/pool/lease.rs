//! Lease API for pooled wireframe clients.
//!
//! A lease holds one admission permit against a physical socket. Each
//! operation checks out the underlying warm socket from that slot, executes
//! the requested client method, then returns the socket to `bb8`.
//!
//! The lease also owns the socket's *response stream* for as long as it lives.
//! A split-phase `send` leaves one reply on the wire; the socket may only be
//! handed to someone else after that reply has been read. Cancellation is the
//! hard case: dropping an in-flight operation, or the lease itself, must
//! invalidate the socket rather than let a late reply reach the next lease.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::{Mutex, OwnedSemaphorePermit};

use super::{
    client_pool::PoolSchedulerControl,
    core::PoolCore,
    managed::ManagedClientConnection,
    slot::{PoolSlot, SlotConnection},
};
use crate::{
    app::Packet,
    client::ClientError,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

/// How a completed lease operation changes its split-phase response state.
#[derive(Clone, Copy)]
enum PendingResponseChange {
    /// A split-phase send may leave one response on the socket.
    Add,
    /// A split-phase receive consumed one response.
    Remove,
    /// A whole request-response operation owns its request and reply pair.
    None,
}

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
    /// Number of split-phase responses this lease has not consumed yet.
    pending_responses: AtomicUsize,
    /// Protocol gate retained until this lease consumes its pending responses.
    ///
    /// The admission permit held by `_permit` bounds concurrency across the
    /// pool; this gate is what keeps a second lease from checking out a socket
    /// that still owes this lease a reply.
    pending_response_permit: Mutex<Option<OwnedSemaphorePermit>>,
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
            pending_responses: AtomicUsize::new(0),
            pending_response_permit: Mutex::new(None),
        })
    }

    /// Resolve the selected slot through the core's checked internal accessor.
    pub(crate) fn slot(&self) -> Option<&PoolSlot<S, P, C>> { self.core.slot(self.slot_index) }

    /// Classify transport errors that make a pooled connection unsafe to reuse.
    const fn should_recycle(err: &ClientError) -> bool { err.should_recycle_connection() }

    /// Run one operation against a checked-out connection and mark bad sockets.
    async fn dispatch_on_connection<R>(
        &self,
        pending_response_change: PendingResponseChange,
        operation: impl AsyncFnOnce(&mut ManagedClientConnection<S, C>) -> Result<R, ClientError>,
    ) -> Result<R, ClientError> {
        let slot = self.slot().ok_or_else(ClientError::disconnected)?;
        let mut pending_response_permit = self.pending_response_permit.lock().await;
        self.admit_operation(&mut pending_response_permit, pending_response_change)
            .await?;

        let mut connection = match slot.checkout(&self.pending_responses).await {
            Ok(connection) => connection,
            Err(error) => {
                // A failed checkout means this lease cannot read a reply it is
                // still owed. Retire the socket before the gate returns, or the
                // late reply could reach whichever lease checks out next. This
                // also clears the count, so the next operation reacquires the
                // gate instead of running without a reservation.
                if self.pending_responses.load(Ordering::Acquire) > 0 {
                    self.discard_connection_with_pending_response();
                }
                // The gate must return to the slot even when checkout fails;
                // otherwise this lease would strand the socket it never used.
                drop(pending_response_permit.take());
                return Err(error);
            }
        };

        let result = operation(&mut connection).await;
        // Reaching this line means the operation future ran to completion, so
        // the exchange is not half-finished and the guard may be disarmed.
        connection.disarm();
        let keeps_socket_busy =
            self.finish_operation(&mut connection, &result, pending_response_change);
        drop(connection);
        Self::finish_protocol_permit(&mut pending_response_permit, keeps_socket_busy);
        result
    }

    /// Reserve the socket before checkout while an earlier response is pending.
    ///
    /// A lease that is still owed a reply may only run the matching `receive`.
    /// Anything else would read that lease's reply, so the socket is discarded
    /// and the operation fails instead.
    async fn admit_operation(
        &self,
        pending_response_permit: &mut Option<OwnedSemaphorePermit>,
        pending_response_change: PendingResponseChange,
    ) -> Result<(), ClientError> {
        let has_pending_response = self.pending_responses.load(Ordering::Acquire) > 0;
        let is_whole_exchange = matches!(pending_response_change, PendingResponseChange::None);
        if has_pending_response && is_whole_exchange {
            self.discard_connection_with_pending_response();
            drop(pending_response_permit.take());
            return Err(ClientError::disconnected());
        }

        // Between a split-phase send and its receive the lease keeps the gate,
        // so its own `receive` must proceed without taking it again.
        if !has_pending_response {
            let slot = self.slot().ok_or_else(ClientError::disconnected)?;
            let permit = slot.acquire_protocol_permit().await?;
            *pending_response_permit = Some(permit);
        }
        Ok(())
    }

    /// Update pending-response state and report whether the socket stays busy.
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
            return false;
        }

        if result.is_ok() {
            self.record_response_change(change);
        }
        self.pending_responses.load(Ordering::Acquire) > 0
    }

    /// Release the gate unless the socket must stay reserved for a reply.
    fn finish_protocol_permit(
        pending_response_permit: &mut Option<OwnedSemaphorePermit>,
        keeps_socket_busy: bool,
    ) {
        if !keeps_socket_busy {
            drop(pending_response_permit.take());
        }
    }

    /// Record whether a split-phase exchange still has an unread response.
    fn record_response_change(&self, change: PendingResponseChange) {
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

    /// Retire the socket because this lease will not read its reply.
    fn discard_connection_with_pending_response(&self) {
        if let Some(slot) = self.slot() {
            slot.discard_next_checkout();
        }
        self.pending_responses.store(0, Ordering::Release);
        crate::metrics::inc_pool_pending_response_discard();
        tracing::debug!(
            action = "discard_connection",
            pool_operation = "split_phase_exchange",
            reason = "response_pending",
            "discarding pooled client connection with an unread response"
        );
    }

    /// Run a whole request-response exchange under one protocol reservation.
    ///
    /// Both the request and its reply belong to this operation, so it can
    /// never leave the socket holding an unread response. Split-phase sends
    /// use [`Self::dispatch_on_connection`] directly instead, because they
    /// keep the protocol gate until the matching receive consumes the reply.
    async fn dispatch_whole_exchange<R>(
        &self,
        operation: impl AsyncFnOnce(&mut ManagedClientConnection<S, C>) -> Result<R, ClientError>,
    ) -> Result<R, ClientError> {
        self.dispatch_on_connection(PendingResponseChange::None, operation)
            .await
    }

    /// Send one message over the leased socket.
    ///
    /// A successful split-phase send leaves one response pending until this
    /// lease receives it. Dropping the lease before then discards the socket.
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
        self.dispatch_whole_exchange(async |conn| conn.client_mut()?.call(request).await)
            .await
    }

    /// Send one envelope and return the correlation ID used.
    ///
    /// A successful split-phase send leaves one response pending until this
    /// lease receives it. Dropping the lease before then discards the socket.
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
        self.dispatch_on_connection(PendingResponseChange::Remove, async |conn| {
            conn.client_mut()?.receive_envelope().await
        })
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
        self.dispatch_whole_exchange(async |conn| conn.client_mut()?.call_correlated(request).await)
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
        // Mark the slot before releasing capacity so no waiter can claim a
        // socket that is still owed this lease's reply.
        if self.pending_responses.load(Ordering::Acquire) > 0 {
            self.discard_connection_with_pending_response();
        }

        if let Some(control) = &self.release_control {
            control
                .scheduler
                .notify_capacity_available(Arc::clone(&self.core), Arc::clone(control));
        }
    }
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;
