//! Byte-accounted request body channel.
//!
//! The public [`body_channel`](super::body_channel) bounds a body by chunk
//! count only. The connection runtime also needs to bound the *bytes* queued
//! for a handler by the same aggregate cap that protects assembly and retained
//! fragments, otherwise a protocol using large chunks could exceed the
//! configured budget before the chunk-count limit applies.
//!
//! This module adds that account without changing the public channel:
//!
//! - every queued chunk carries a [`QueuedBytesGuard`] that removes its length from a shared
//!   counter when the consumer takes the chunk, or when the channel drops it;
//! - admission reserves the chunk's bytes in that counter in the same atomic step that decides they
//!   fit, so a producer holds its room while it waits for channel capacity and a second producer
//!   sharing the account cannot be granted the same space;
//! - a producer waits for capacity when the queue is non-empty and the combined retained bytes
//!   would exceed the cap; and
//! - a single chunk is always admitted to an empty queue. Without that exception a chunk larger
//!   than the remaining budget could never make progress: the producer would wait for space that
//!   only a consumer could create, but the consumer would never receive the chunk. The caller's
//!   hard-cap policy still observes the queued bytes afterwards.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::{Notify, mpsc};

use super::RequestBodyStream;

/// Bytes currently queued for one connection's streaming request bodies.
///
/// Cloning shares the same counter, so the connection can read it while a
/// sender admits another chunk.
#[derive(Clone, Debug, Default)]
pub(crate) struct QueuedBytes {
    /// Total bytes held by queued [`BodyChunk`] values.
    bytes: Arc<AtomicUsize>,
    /// Wakes a producer after the consumer releases queued bytes.
    released: Arc<Notify>,
}

impl QueuedBytes {
    /// Create an empty account.
    pub(crate) fn new() -> Self { Self::default() }

    /// Return the number of bytes currently queued.
    pub(crate) fn queued(&self) -> usize { self.bytes.load(Ordering::SeqCst) }

    /// Reserve `len` bytes for a chunk that is about to be queued.
    ///
    /// The reservation *is* the admission decision: the rule is evaluated
    /// against the value this call then installs, so a producer that is
    /// granted room already holds it. Checking the counter and charging it in
    /// two steps would let two producers that share an account both pass the
    /// check and then each charge, overshooting `limit` by the chunk already
    /// waiting for channel capacity.
    ///
    /// Returns `None` when the chunk does not fit beside the bytes already
    /// queued or reserved and the bytes retained elsewhere.
    #[must_use]
    fn try_reserve(
        &self,
        len: usize,
        other_retained: usize,
        limit: usize,
    ) -> Option<QueuedBytesGuard> {
        let mut queued = self.queued();
        loop {
            if !fits_within_limit(queued, other_retained, len, limit) {
                return None;
            }
            let reserved = queued.saturating_add(len);
            match self
                .bytes
                .compare_exchange(queued, reserved, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(self.guard(len)),
                // Another producer charged or released bytes between the read
                // and the swap, so the rule must be evaluated again against
                // the total it left behind.
                Err(observed) => queued = observed,
            }
        }
    }

    /// Build the guard that releases `len` bytes already charged here.
    ///
    /// Used for a reservation, whose bytes [`Self::try_reserve`] added, and
    /// for a value that carries no bytes at all, such as a terminal error.
    fn guard(&self, len: usize) -> QueuedBytesGuard {
        QueuedBytesGuard {
            bytes: Arc::clone(&self.bytes),
            released: Arc::clone(&self.released),
            len,
        }
    }
}

/// Return whether a chunk may join an account that already holds `queued`.
///
/// An empty account admits one chunk whatever its size: the documented
/// exception that lets a chunk larger than the remaining budget make progress.
const fn fits_within_limit(
    queued: usize,
    other_retained: usize,
    chunk_len: usize,
    limit: usize,
) -> bool {
    queued == 0
        || queued
            .saturating_add(other_retained)
            .saturating_add(chunk_len)
            <= limit
}

/// Guard that releases charged bytes exactly once when dropped.
#[derive(Debug)]
struct QueuedBytesGuard {
    /// Shared counter charged when the guard was created.
    bytes: Arc<AtomicUsize>,
    /// Waker used to resume a producer waiting for capacity.
    released: Arc<Notify>,
    /// Byte count charged by this guard.
    len: usize,
}

impl Drop for QueuedBytesGuard {
    fn drop(&mut self) {
        self.bytes.fetch_sub(self.len, Ordering::SeqCst);
        self.released.notify_waiters();
    }
}

/// One queued body value plus its byte-account guard.
#[derive(Debug)]
struct BodyChunk {
    /// Bytes or terminal error delivered to the handler.
    item: Result<Bytes, io::Error>,
    /// Released when the consumer takes this chunk or the channel drops it.
    _guard: QueuedBytesGuard,
}

/// Sender that charges queued bytes before handing a chunk to the channel.
#[derive(Clone, Debug)]
pub(crate) struct AccountedBodySender {
    /// Bounded channel shared with the handler's body stream.
    tx: mpsc::Sender<BodyChunk>,
    /// Connection-wide queued-byte account.
    queued: QueuedBytes,
}

impl AccountedBodySender {
    /// Admit and send one body chunk.
    ///
    /// `other_retained` is the number of bytes the connection retains outside
    /// this queue (assembly partials, retained fragments and parked messages).
    /// Admission waits while the queue is non-empty and adding the chunk would
    /// exceed `limit`. The empty-queue exception is documented at module level.
    ///
    /// The chunk's bytes are reserved by [`Self::admit`] and stay charged while
    /// the send waits for channel capacity. The reservation becomes the chunk's
    /// own guard once it is queued, and is released if the consumer disappears
    /// or the send is dropped first.
    ///
    /// # Errors
    ///
    /// Returns [`SendChunkError::Closed`] when the handler dropped its body.
    pub(crate) async fn send_chunk(
        self,
        chunk: Bytes,
        other_retained: usize,
        limit: usize,
    ) -> Result<(), SendChunkError> {
        let reservation = self.admit(chunk.len(), other_retained, limit).await?;
        let permit = self
            .tx
            .reserve()
            .await
            .map_err(|_| SendChunkError::Closed)?;
        permit.send(BodyChunk {
            item: Ok(chunk),
            _guard: reservation,
        });
        Ok(())
    }

    /// Send a terminal error to the consumer, bypassing byte admission.
    ///
    /// Errors carry no body bytes, but they still need a channel slot so they
    /// cannot overtake queued chunks.
    ///
    /// # Errors
    ///
    /// Returns [`SendClosed`] when the handler dropped its body.
    pub(crate) async fn send_terminal(self, error: io::Error) -> Result<(), SendClosed> {
        let permit = self.tx.reserve().await.map_err(|_| SendClosed)?;
        let guard = self.queued.guard(0);
        permit.send(BodyChunk {
            item: Err(error),
            _guard: guard,
        });
        Ok(())
    }

    /// Reserve room for the chunk, waiting while the account is full.
    ///
    /// The reservation is returned to the caller rather than kept here, so the
    /// bytes stay charged while the chunk waits for channel capacity and are
    /// released if that wait is abandoned before the chunk is queued.
    async fn admit(
        &self,
        chunk_len: usize,
        other_retained: usize,
        limit: usize,
    ) -> Result<QueuedBytesGuard, SendChunkError> {
        loop {
            // Register interest before reading the counter so a release
            // between the check and the await cannot be lost.
            let released = self.queued.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();

            if let Some(reservation) = self.queued.try_reserve(chunk_len, other_retained, limit) {
                return Ok(reservation);
            }
            if self.tx.is_closed() {
                return Err(SendChunkError::Closed);
            }
            // A closed receiver never notifies the byte account, so waiting on
            // a release alone would park this send until some other body
            // released bytes or the handler resolved.
            tokio::select! {
                () = &mut released => {}
                () = self.tx.closed() => return Err(SendChunkError::Closed),
            }
        }
    }
}

/// Admission failure for a body chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SendChunkError {
    /// The handler dropped its body stream.
    Closed,
}

/// Terminal delivery failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SendClosed;

/// Create a byte-accounted bounded request-body channel.
pub(crate) fn accounted_body_channel(
    capacity: usize,
    queued: QueuedBytes,
) -> (AccountedBodySender, RequestBodyStream) {
    let (tx, rx) = mpsc::channel(capacity);
    let sender = AccountedBodySender { tx, queued };
    (sender, accounted_stream(rx))
}

/// Wrap the receiver so each chunk releases its queued-byte guard on yield.
fn accounted_stream(rx: mpsc::Receiver<BodyChunk>) -> RequestBodyStream {
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|chunk| {
        let BodyChunk { item, _guard } = chunk;
        item
    });
    Box::pin(stream)
}
