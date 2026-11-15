//! Streaming entry points on [`MessageAssemblyState`].
//!
//! These methods keep streaming partials in their own map so they never
//! contribute to the aggregate assembly budgets, and report evicted streaming
//! keys separately so the connection can fail a waiting handler's body.

use std::time::Instant;

use super::{
    ContinuationFrameHeader,
    MessageAssemblyError,
    MessageKey,
    MessageSeriesError,
    StreamingContinuation,
    StreamingFailure,
    StreamingFirstFrame,
    streaming::StreamingAssembly,
    validation,
};

/// Keys evicted by [`MessageAssemblyState::purge_expired_detailed`].
///
/// Streaming keys are reported separately so the connection can fail the
/// handler's body with a timeout instead of leaving it waiting forever.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PurgedAssemblies {
    /// Buffered assemblies that were discarded.
    buffered: Vec<MessageKey>,
    /// Streaming assemblies whose consumer must be notified.
    streaming: Vec<MessageKey>,
}

impl PurgedAssemblies {
    /// Return the evicted buffered assembly keys.
    #[must_use]
    pub fn buffered(&self) -> &[MessageKey] { &self.buffered }

    /// Return the evicted streaming assembly keys.
    #[must_use]
    pub fn streaming(&self) -> &[MessageKey] { &self.streaming }

    /// Return whether no assembly was evicted.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.buffered.is_empty() && self.streaming.is_empty() }
}

impl super::MessageAssemblyState {
    /// Admit a first frame for immediate delivery to a streaming route.
    ///
    /// A single-frame message is returned without inserting any partial state.
    /// A multi-frame message inserts a streaming partial that retains only its
    /// ordering and byte-count state, so it contributes nothing to
    /// [`total_buffered_bytes`](Self::total_buffered_bytes).
    ///
    /// # Errors
    ///
    /// Returns [`MessageAssemblyError::DuplicateFirstFrame`] when the key is
    /// already assembling, [`MessageAssemblyError::MessageTooLarge`] when the
    /// declared message cannot fit the per-message cap, or a declared-total
    /// error when the first frame contradicts its declaration.
    pub fn accept_streaming_first_frame<'a>(
        &mut self,
        input: super::FirstFrameInput<'a>,
    ) -> Result<StreamingFirstFrame<'a>, MessageAssemblyError> {
        self.accept_streaming_first_frame_at(input, Instant::now())
    }

    /// Admit a streaming first frame using an explicit clock reading.
    ///
    /// # Errors
    ///
    /// See [`accept_streaming_first_frame`](Self::accept_streaming_first_frame).
    pub fn accept_streaming_first_frame_at<'a>(
        &mut self,
        input: super::FirstFrameInput<'a>,
        now: Instant,
    ) -> Result<StreamingFirstFrame<'a>, MessageAssemblyError> {
        let _purged = self.purge_expired_detailed_at(now);
        let key = input.header.message_key;

        if self.assemblies.contains_key(&key) || self.streaming_assemblies.contains_key(&key) {
            return Err(MessageAssemblyError::DuplicateFirstFrame { key });
        }

        validation::validate_first_frame(input.header, input.body.len())?;

        let declared_body_len = input.header.total_body_len.unwrap_or(input.body.len());
        let total_message_size = declared_body_len.saturating_add(input.metadata.len());
        if total_message_size > self.max_message_size.get() {
            return Err(MessageAssemblyError::MessageTooLarge {
                key,
                attempted: total_message_size,
                limit: self.max_message_size,
            });
        }

        let chunk = input.body;
        let is_last = input.header.is_last;
        let routing = input.routing;
        if !is_last {
            let partial = StreamingAssembly::from_first_frame(&input, chunk.len(), now);
            self.streaming_assemblies.insert(key, partial);
        }
        Ok(StreamingFirstFrame {
            key,
            routing,
            metadata: input.metadata,
            chunk,
            is_last,
        })
    }

    /// Admit a continuation frame for an existing streaming message.
    ///
    /// # Errors
    ///
    /// Returns a [`StreamingFailure`] when continuity, size, or declared-total
    /// validation fails. `is_terminal` reports whether the partial was removed
    /// and the handler's stream must be closed.
    pub fn accept_streaming_continuation<'a>(
        &mut self,
        header: &ContinuationFrameHeader,
        body: &'a [u8],
    ) -> Result<StreamingContinuation<'a>, StreamingFailure> {
        self.accept_streaming_continuation_at(header, body, Instant::now())
    }

    /// Admit a streaming continuation using an explicit clock reading.
    ///
    /// # Errors
    ///
    /// See [`accept_streaming_continuation`](Self::accept_streaming_continuation).
    pub fn accept_streaming_continuation_at<'a>(
        &mut self,
        header: &ContinuationFrameHeader,
        body: &'a [u8],
        now: Instant,
    ) -> Result<StreamingContinuation<'a>, StreamingFailure> {
        let _purged = self.purge_expired_detailed_at(now);
        let key = header.message_key;

        if header.body_len != body.len() {
            return Err(StreamingFailure::new(
                key,
                MessageAssemblyError::Series(MessageSeriesError::ContinuationBodyLengthMismatch {
                    key,
                    header_len: header.body_len,
                    actual_len: body.len(),
                }),
                false,
            ));
        }

        let Some(partial) = self.streaming_assemblies.get_mut(&key) else {
            return Err(StreamingFailure::new(
                key,
                MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame { key }),
                true,
            ));
        };

        let outcome = partial.accept_continuation(header, body, self.max_message_size);
        match outcome {
            Ok(Some(chunk)) => {
                let is_last = chunk.is_last;
                if is_last {
                    self.streaming_assemblies.remove(&key);
                }
                Ok(StreamingContinuation::Chunk {
                    key,
                    chunk: chunk.body,
                    is_last,
                })
            }
            Ok(None) => {
                let complete = partial.is_complete();
                if complete {
                    self.streaming_assemblies.remove(&key);
                }
                Ok(StreamingContinuation::Discarded { key })
            }
            Err(failure) => {
                if failure.is_terminal() {
                    self.streaming_assemblies.remove(&key);
                }
                Err(failure)
            }
        }
    }

    /// Return whether `key` is currently assembling for a streaming route.
    #[must_use]
    pub fn is_streaming_key(&self, key: MessageKey) -> bool {
        self.streaming_assemblies.contains_key(&key)
    }

    /// Switch `key` into discard mode so remaining frames are validated but
    /// not delivered.
    ///
    /// Returns `false` when no streaming assembly exists for the key.
    pub fn discard_streaming(&mut self, key: MessageKey) -> bool {
        self.streaming_assemblies
            .get_mut(&key)
            .is_some_and(|partial| {
                partial.begin_discard();
                true
            })
    }

    /// Number of streaming assemblies currently tracked.
    #[must_use]
    pub fn streaming_count(&self) -> usize { self.streaming_assemblies.len() }

    /// Remove expired buffered and streaming assemblies.
    #[must_use]
    pub fn purge_expired_detailed(&mut self) -> PurgedAssemblies {
        self.purge_expired_detailed_at(Instant::now())
    }

    /// Remove expired assemblies using an explicit clock reading.
    #[must_use]
    pub fn purge_expired_detailed_at(&mut self, now: Instant) -> PurgedAssemblies {
        let timeout = self.timeout;
        let mut purged = PurgedAssemblies::default();

        self.assemblies.retain(|key, partial| {
            let expired = now.saturating_duration_since(partial.started_at) >= timeout;
            if expired {
                purged.buffered.push(*key);
            }
            !expired
        });
        self.streaming_assemblies.retain(|key, partial| {
            let expired = now.saturating_duration_since(partial.started_at) >= timeout;
            if expired {
                purged.streaming.push(*key);
            }
            !expired
        });

        purged
    }
}
