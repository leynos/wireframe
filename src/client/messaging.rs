//! Envelope-based messaging API for the wireframe client.
//!
//! This module provides methods for sending and receiving envelopes with
//! automatic correlation ID generation and validation.

use std::{sync::atomic::Ordering, time::Instant};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tracing::Instrument;

use super::{
    ClientError,
    runtime::ClientStream,
    tracing_helpers::{call_correlated_span, emit_timing_event, receive_span, send_envelope_span},
};
use crate::{
    app::Packet,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

/// Extension trait providing envelope-based messaging methods.
///
/// These methods are implemented as an extension trait to keep the main
/// `WireframeClient` struct focused on core functionality while providing
/// the correlation-aware APIs in a separate module.
impl<S, T, C> super::WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Generate the next unique correlation identifier for this connection.
    ///
    /// Correlation identifiers are generated sequentially starting from 1.
    /// Each call returns a unique value within this client instance.
    ///
    /// # Concurrency
    ///
    /// This method is thread-safe: the counter uses `AtomicU64` with
    /// `Ordering::Relaxed`. Relaxed ordering is sufficient here because:
    ///
    /// 1. The only guarantee required is uniqueness of IDs, not ordering relative to other memory
    ///    operations.
    /// 2. The atomic `fetch_add` operation itself is always atomic regardless of memory ordering,
    ///    ensuring no two calls ever return the same value.
    /// 3. There are no other memory operations that need to be synchronised with the counter
    ///    increment.
    ///
    /// The counter is safe for cross-thread access, though typical usage
    /// involves a single task holding `&self` at a time.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::client::{ClientError, WireframeClient};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let client = WireframeClient::builder().connect(addr).await?;
    /// let id1 = client.next_correlation_id();
    /// let id2 = client.next_correlation_id();
    /// assert_ne!(id1, id2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn next_correlation_id(&self) -> u64 {
        self.correlation_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Send an envelope to the peer using the configured serializer.
    ///
    /// If the envelope does not have a correlation ID set, one is automatically
    /// generated and stamped on the envelope before sending.
    ///
    /// Returns the correlation ID that was used (either the existing one or
    /// the auto-generated one).
    ///
    /// If an error hook is registered, it is invoked before the error is
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if serialization or transport I/O fails.
    /// Transport failures are surfaced through
    /// [`crate::WireframeError::Io`] within
    /// [`ClientError::Wireframe`](super::ClientError::Wireframe).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{
    ///     app::Envelope,
    ///     client::{ClientError, WireframeClient},
    ///     correlation::CorrelatableFrame,
    /// };
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    ///
    /// // Correlation ID will be auto-generated
    /// let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    /// let correlation_id = client.send_envelope(envelope).await?;
    /// assert!(correlation_id > 0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_envelope<P>(&mut self, mut envelope: P) -> Result<u64, ClientError>
    where
        P: Packet + EncodeWith<S>,
    {
        // Check once whether correlation ID is present.
        let existing = envelope.correlation_id();
        let correlation_id = existing.unwrap_or_else(|| self.next_correlation_id());

        // Set correlation ID in-place if it was auto-generated.
        // This preserves any custom fields in the Packet implementation.
        if existing.is_none() {
            envelope.set_correlation_id(Some(correlation_id));
        }

        let mut bytes = match self.serializer.serialize(&envelope) {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::Serialize(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        let span = send_envelope_span(&self.tracing_config, correlation_id, bytes.len());
        let timing_start = self.tracing_config.send_timing.then(Instant::now);
        self.invoke_before_send_hooks(&mut bytes);
        let send_result = async {
            let result = self.framed.send(Bytes::from(bytes)).await;
            emit_timing_event(timing_start);
            result
        }
        .instrument(span)
        .await;
        if let Err(e) = send_result {
            let err = ClientError::from(e);
            self.invoke_error_hook(&err).await;
            return Err(err);
        }
        Ok(correlation_id)
    }

    /// Receive the next envelope from the peer.
    ///
    /// If an error hook is registered, it is invoked before the error is
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the connection closes, decoding fails, or I/O
    /// errors occur. Transport failures are surfaced through
    /// [`crate::WireframeError::Io`], while decode failures are surfaced
    /// through [`crate::WireframeError::Protocol`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{
    ///     app::Envelope,
    ///     client::{ClientError, WireframeClient},
    ///     correlation::CorrelatableFrame,
    /// };
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    /// let envelope: Envelope = client.receive_envelope().await?;
    /// let _correlation_id = envelope.correlation_id();
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_envelope<P>(&mut self) -> Result<P, ClientError>
    where
        P: Packet + DecodeWith<S>,
    {
        self.receive_internal().await
    }

    /// Send an envelope and await a correlated response.
    ///
    /// This method auto-generates a correlation ID for the request (if not
    /// present), sends the envelope, receives the response, and validates that
    /// the response's correlation ID matches the request.
    ///
    /// If an error hook is registered, it is invoked before any error is
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::CorrelationMismatch`] if the response correlation
    /// ID does not match the request. Returns other [`ClientError`] variants
    /// for serialization, decode, or transport failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{
    ///     app::Envelope,
    ///     client::{ClientError, WireframeClient},
    /// };
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    ///
    /// let request = Envelope::new(1, None, vec![1, 2, 3]);
    /// let response: Envelope = client.call_correlated(request).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call_correlated<P>(&mut self, request: P) -> Result<P, ClientError>
    where
        P: Packet + EncodeWith<S> + DecodeWith<S>,
    {
        let span = call_correlated_span(&self.tracing_config);
        let timing_start = self.tracing_config.call_timing.then(Instant::now);

        self.call_correlated_inner(request, &span, timing_start)
            .instrument(span.clone())
            .await
    }

    /// Execute the correlated call within an active span.
    async fn call_correlated_inner<P>(
        &mut self,
        request: P,
        span: &tracing::Span,
        timing_start: Option<Instant>,
    ) -> Result<P, ClientError>
    where
        P: Packet + EncodeWith<S> + DecodeWith<S>,
    {
        let correlation_id = match self.send_envelope(request).await {
            Ok(id) => id,
            Err(err) => {
                // Error hook already invoked by send_envelope.
                span.record("result", "err");
                emit_timing_event(timing_start);
                return Err(err);
            }
        };
        span.record("correlation_id", correlation_id);
        let response: P = match self.receive_envelope().await {
            Ok(response) => response,
            Err(err) => {
                // Error hook already invoked by receive_internal.
                span.record("result", "err");
                emit_timing_event(timing_start);
                return Err(err);
            }
        };

        // Validate correlation ID matches.
        let response_correlation_id = response.correlation_id();
        if response_correlation_id != Some(correlation_id) {
            let err = ClientError::CorrelationMismatch {
                expected: Some(correlation_id),
                received: response_correlation_id,
            };
            return Err(self.traced_error(span, timing_start, err).await);
        }

        Self::traced_ok(span, timing_start);
        Ok(response)
    }

    /// Internal helper for receiving and deserializing a frame.
    pub(crate) async fn receive_internal<R: DecodeWith<S>>(&mut self) -> Result<R, ClientError> {
        let span = receive_span(&self.tracing_config);
        let timing_start = self.tracing_config.receive_timing.then(Instant::now);

        self.receive_frame(&span, timing_start)
            .instrument(span.clone())
            .await
    }

    /// Receive and deserialize a single frame within an active span.
    async fn receive_frame<R: DecodeWith<S>>(
        &mut self,
        span: &tracing::Span,
        timing_start: Option<Instant>,
    ) -> Result<R, ClientError> {
        let Some(frame) = self.framed.next().await else {
            let err = ClientError::disconnected();
            return Err(self.traced_error(span, timing_start, err).await);
        };
        let mut bytes = match frame {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::from(e);
                return Err(self.traced_error(span, timing_start, err).await);
            }
        };
        span.record("frame.bytes", bytes.len());
        self.invoke_after_receive_hooks(&mut bytes);
        let (message, _consumed) = match self.serializer.deserialize(&bytes) {
            Ok(result) => result,
            Err(e) => {
                let err = ClientError::decode(e);
                return Err(self.traced_error(span, timing_start, err).await);
            }
        };
        Self::traced_ok(span, timing_start);
        Ok(message)
    }

    /// Invoke the error hook if one is registered.
    pub(crate) async fn invoke_error_hook(&self, error: &ClientError) {
        if let Some(ref handler) = self.on_error {
            handler(error).await;
        }
    }

    /// Record a span result, emit timing, invoke the error hook, and return
    /// the error for early-return paths.
    pub(crate) async fn traced_error(
        &self,
        span: &tracing::Span,
        timing_start: Option<Instant>,
        err: ClientError,
    ) -> ClientError {
        span.record("result", "err");
        emit_timing_event(timing_start);
        self.invoke_error_hook(&err).await;
        err
    }

    /// Record a successful span result and emit timing.
    pub(crate) fn traced_ok(span: &tracing::Span, timing_start: Option<Instant>) {
        span.record("result", "ok");
        emit_timing_event(timing_start);
    }

    /// Invoke all registered before-send hooks in registration order.
    pub(crate) fn invoke_before_send_hooks(&self, bytes: &mut Vec<u8>) {
        for hook in &self.request_hooks.before_send {
            hook(bytes);
        }
    }

    /// Invoke all registered after-receive hooks in registration order.
    pub(crate) fn invoke_after_receive_hooks(&self, bytes: &mut bytes::BytesMut) {
        for hook in &self.request_hooks.after_receive {
            hook(bytes);
        }
    }
}
