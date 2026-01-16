//! Envelope-based messaging API for the wireframe client.
//!
//! This module provides methods for sending and receiving envelopes with
//! automatic correlation ID generation and validation.

use std::sync::atomic::Ordering;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};

use super::{ClientError, runtime::ClientStream};
use crate::{app::Packet, message::Message, serializer::Serializer};

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
    /// This method uses `Ordering::Relaxed` for the atomic increment. Relaxed
    /// ordering is sufficient here because:
    ///
    /// 1. The only guarantee required is uniqueness of IDs, not ordering relative to other memory
    ///    operations.
    /// 2. The atomic `fetch_add` operation itself is always atomic regardless of memory ordering,
    ///    ensuring no two calls ever return the same value.
    /// 3. There are no other memory operations that need to be synchronised with the counter
    ///    increment.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
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
    /// Returns [`ClientError`] if serialization or I/O fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
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
    pub async fn send_envelope<P: Packet>(&mut self, mut envelope: P) -> Result<u64, ClientError> {
        // Check once whether correlation ID is present.
        let existing = envelope.correlation_id();
        let correlation_id = existing.unwrap_or_else(|| self.next_correlation_id());

        // Set correlation ID in-place if it was auto-generated.
        // This preserves any custom fields in the Packet implementation.
        if existing.is_none() {
            envelope.set_correlation_id(Some(correlation_id));
        }

        let bytes = match self.serializer.serialize(&envelope) {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::Serialize(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        if let Err(e) = self.framed.send(Bytes::from(bytes)).await {
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
    /// errors occur.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
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
    pub async fn receive_envelope<P: Packet>(&mut self) -> Result<P, ClientError> {
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
    /// for serialization, deserialization, or I/O failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
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
    pub async fn call_correlated<P: Packet>(&mut self, request: P) -> Result<P, ClientError> {
        let correlation_id = self.send_envelope(request).await?;
        let response: P = self.receive_envelope().await?;

        // Validate correlation ID matches.
        let response_correlation_id = response.correlation_id();
        if response_correlation_id != Some(correlation_id) {
            let err = ClientError::CorrelationMismatch {
                expected: Some(correlation_id),
                received: response_correlation_id,
            };
            self.invoke_error_hook(&err).await;
            return Err(err);
        }

        Ok(response)
    }

    /// Internal helper for receiving and deserializing a frame.
    pub(crate) async fn receive_internal<R: Message>(&mut self) -> Result<R, ClientError> {
        let Some(frame) = self.framed.next().await else {
            let err = ClientError::Disconnected;
            self.invoke_error_hook(&err).await;
            return Err(err);
        };
        let bytes = match frame {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::from(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        let (message, _consumed) = match self.serializer.deserialize(&bytes) {
            Ok(result) => result,
            Err(e) => {
                let err = ClientError::Deserialize(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        Ok(message)
    }

    /// Invoke the error hook if one is registered.
    pub(crate) async fn invoke_error_hook(&self, error: &ClientError) {
        if let Some(ref handler) = self.on_error {
            handler(error).await;
        }
    }
}
