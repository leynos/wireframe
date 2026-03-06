//! Lease API for pooled wireframe clients.
//!
//! A lease holds one admission permit against a physical socket. Each
//! operation checks out the underlying warm socket from that slot, executes
//! the requested client method, then returns the socket to `bb8`.

use std::sync::Arc;

use tokio::sync::OwnedSemaphorePermit;

use super::slot::PoolSlot;
use crate::{
    WireframeError,
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
    slot: Arc<PoolSlot<S, P, C>>,
    _permit: OwnedSemaphorePermit,
}

macro_rules! dispatch_on_connection {
    ($self:ident, | $conn:ident | $op:expr) => {{
        let mut $conn = $self.slot.checkout().await?;
        let result = $op.await;
        if let Err(err) = &result
            && Self::should_recycle(err)
        {
            $conn.mark_broken();
        }
        result
    }};
}

impl<S, P, C> PooledClientLease<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) fn new(slot: Arc<PoolSlot<S, P, C>>, permit: OwnedSemaphorePermit) -> Self {
        Self {
            slot,
            _permit: permit,
        }
    }

    fn should_recycle(err: &ClientError) -> bool {
        matches!(err, ClientError::Wireframe(WireframeError::Io(_)))
    }

    /// Send one message over the leased socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, serialization, or transport I/O
    /// fails.
    pub async fn send<M: EncodeWith<S>>(&self, message: &M) -> Result<(), ClientError> {
        dispatch_on_connection!(self, |conn| conn.send(message))
    }

    /// Receive one message from the leased socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when checkout, decode, or transport I/O fails.
    pub async fn receive<M: DecodeWith<S>>(&self) -> Result<M, ClientError> {
        dispatch_on_connection!(self, |conn| conn.receive())
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
        dispatch_on_connection!(self, |conn| conn.call(request))
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
        dispatch_on_connection!(self, |conn| conn.send_envelope(envelope))
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
        dispatch_on_connection!(self, |conn| conn.receive_envelope())
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
        dispatch_on_connection!(self, |conn| conn.call_correlated(request))
    }
}
