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

    /// Charge `len` bytes to the account, returning a guard that releases them.
    fn charge(&self, len: usize) -> QueuedBytesGuard {
        self.bytes.fetch_add(len, Ordering::SeqCst);
        QueuedBytesGuard {
            bytes: Arc::clone(&self.bytes),
            released: Arc::clone(&self.released),
            len,
        }
    }
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
    /// # Errors
    ///
    /// Returns [`SendChunkError::Closed`] when the handler dropped its body.
    pub(crate) async fn send_chunk(
        self,
        chunk: Bytes,
        other_retained: usize,
        limit: usize,
    ) -> Result<(), SendChunkError> {
        self.admit(chunk.len(), other_retained, limit).await?;
        let permit = self
            .tx
            .reserve()
            .await
            .map_err(|_| SendChunkError::Closed)?;
        let guard = self.queued.charge(chunk.len());
        permit.send(BodyChunk {
            item: Ok(chunk),
            _guard: guard,
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
        let guard = self.queued.charge(0);
        permit.send(BodyChunk {
            item: Err(error),
            _guard: guard,
        });
        Ok(())
    }

    /// Wait until the chunk can be queued without exceeding the byte cap.
    async fn admit(
        &self,
        chunk_len: usize,
        other_retained: usize,
        limit: usize,
    ) -> Result<(), SendChunkError> {
        loop {
            // Register interest before reading the counter so a release
            // between the check and the await cannot be lost.
            let released = self.queued.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();

            let queued = self.queued.queued();
            let fits = queued
                .saturating_add(other_retained)
                .saturating_add(chunk_len)
                <= limit;
            if queued == 0 || fits {
                return Ok(());
            }
            if self.tx.is_closed() {
                return Err(SendChunkError::Closed);
            }
            released.await;
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
