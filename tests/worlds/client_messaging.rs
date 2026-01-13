//! Test world for client messaging scenarios with correlation ID support.
#![cfg(not(loom))]

use std::net::SocketAddr;

use bytes::Bytes;
use cucumber::World;
use futures::{SinkExt, StreamExt};
use log::warn;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    BincodeSerializer,
    Serializer,
    app::{Envelope, Packet},
    client::{ClientError, WireframeClient},
    rewind_stream::RewindStream,
};

use super::TestResult;

/// Server mode for testing various correlation ID scenarios.
#[derive(Debug, Clone, Copy, Default)]
pub enum ServerMode {
    /// Echo envelopes back with the same correlation ID.
    #[default]
    Echo,
    /// Return envelopes with a different (mismatched) correlation ID.
    Mismatch,
}

/// Test world for client messaging scenarios.
#[derive(Debug, Default, World)]
pub struct ClientMessagingWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>>,
    envelope: Option<Envelope>,
    sent_correlation_ids: Vec<u64>,
    /// The last response received from the server.
    pub response: Option<Envelope>,
    last_error: Option<ClientError>,
    /// Expected message ID for response verification.
    expected_message_id: Option<u32>,
    /// Expected payload for response verification.
    expected_payload: Option<String>,
}

/// Process a single frame in the echo server.
fn process_frame(mode: ServerMode, bytes: &[u8]) -> Option<Vec<u8>> {
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(bytes).ok()?;

    let response = match mode {
        ServerMode::Echo => envelope,
        ServerMode::Mismatch => {
            let wrong_id = envelope.correlation_id().map(|id| id.wrapping_add(999));
            let parts = envelope.into_parts();
            Envelope::new(parts.id(), wrong_id, parts.payload())
        }
    };

    BincodeSerializer.serialize(&response).ok()
}

impl ClientMessagingWorld {
    /// Start an envelope echo server.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub async fn start_echo_server(&mut self) -> TestResult {
        self.start_server_with_mode(ServerMode::Echo).await
    }

    /// Start a server that returns mismatched correlation IDs.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub async fn start_mismatch_server(&mut self) -> TestResult {
        self.start_server_with_mode(ServerMode::Mismatch).await
    }

    async fn start_server_with_mode(&mut self, mode: ServerMode) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                warn!("client messaging server failed to accept connection");
                return;
            };

            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            run_frame_loop(&mut framed, mode).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Connect a client to the server.
    ///
    /// # Errors
    /// Returns an error if the server has not started or the client fails to connect.
    pub async fn connect_client(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let client = WireframeClient::builder().connect(addr).await?;
        self.client = Some(client);
        Ok(())
    }

    /// Set an envelope without a correlation ID.
    pub fn set_envelope_without_correlation(&mut self) {
        self.envelope = Some(Envelope::new(1, None, vec![1, 2, 3]));
    }

    /// Set an envelope with a specific correlation ID.
    pub fn set_envelope_with_correlation(&mut self, correlation_id: u64) {
        self.envelope = Some(Envelope::new(1, Some(correlation_id), vec![1, 2, 3]));
    }

    /// Set an envelope with a specific message ID and payload.
    pub fn set_envelope_with_payload(&mut self, message_id: u32, payload: &str) {
        self.envelope = Some(Envelope::new(message_id, None, payload.as_bytes().to_vec()));
        self.expected_message_id = Some(message_id);
        self.expected_payload = Some(payload.to_string());
    }

    /// Send the configured envelope and capture the returned correlation ID.
    ///
    /// # Errors
    /// Returns an error if the client is missing or communication fails.
    pub async fn send_envelope(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let envelope = self.envelope.take().ok_or("envelope not configured")?;
        let correlation_id = client.send_envelope(envelope).await?;
        self.sent_correlation_ids.push(correlation_id);
        Ok(())
    }

    /// Call the server with `call_correlated` and capture the response.
    ///
    /// # Errors
    /// Returns an error if the client is missing or communication fails.
    pub async fn call_correlated(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let envelope = self.envelope.take().ok_or("envelope not configured")?;

        match client.call_correlated(envelope).await {
            Ok(response) => {
                self.response = Some(response);
                self.last_error = None;
            }
            Err(err) => {
                self.last_error = Some(err);
                self.response = None;
            }
        }
        Ok(())
    }

    /// Send multiple sequential envelopes and capture all correlation IDs.
    ///
    /// # Errors
    /// Returns an error if the client is missing or communication fails.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "test helper with small count values"
    )]
    pub async fn send_multiple_envelopes(&mut self, count: usize) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        self.sent_correlation_ids.clear();

        for i in 0..count {
            let envelope = Envelope::new(i as u32, None, vec![i as u8]);
            let correlation_id = client.send_envelope(envelope).await?;
            self.sent_correlation_ids.push(correlation_id);

            // Drain the echo response.
            let _: Envelope = client.receive_envelope().await?;
        }
        Ok(())
    }

    /// Get the first sent correlation ID.
    fn get_first_correlation_id(&self) -> TestResult<u64> {
        self.sent_correlation_ids
            .first()
            .copied()
            .ok_or_else(|| "no correlation ID captured".into())
    }

    /// Verify that an auto-generated correlation ID was assigned.
    ///
    /// # Errors
    /// Returns an error if no correlation ID was captured or it is zero.
    pub fn verify_auto_generated_correlation(&self) -> TestResult {
        let id = self.get_first_correlation_id()?;
        if id == 0 {
            return Err("correlation ID should be non-zero".into());
        }
        Ok(())
    }

    /// Verify that the returned correlation ID matches the expected value.
    ///
    /// # Errors
    /// Returns an error if no correlation ID was captured or it doesn't match.
    pub fn verify_correlation_id(&self, expected: u64) -> TestResult {
        let id = self.get_first_correlation_id()?;
        if id != expected {
            return Err(format!("expected correlation ID {expected}, got {id}").into());
        }
        Ok(())
    }

    /// Verify that the response has a matching correlation ID.
    ///
    /// # Errors
    /// Returns an error if no response was captured or it lacks a correlation ID.
    pub fn verify_response_correlation_matches(&self) -> TestResult {
        let response = self.response.as_ref().ok_or("no response captured")?;
        if response.correlation_id().is_none() {
            return Err("response should have correlation ID".into());
        }
        Ok(())
    }

    /// Verify that no `CorrelationMismatch` error occurred.
    ///
    /// # Errors
    /// Returns an error if any error was recorded.
    pub fn verify_no_mismatch_error(&self) -> TestResult {
        if self.last_error.is_some() {
            return Err("unexpected error occurred".into());
        }
        Ok(())
    }

    /// Verify that a `CorrelationMismatch` error occurred.
    ///
    /// # Errors
    /// Returns an error if no mismatch error was recorded or a different error occurred.
    pub fn verify_mismatch_error(&self) -> TestResult {
        match &self.last_error {
            Some(ClientError::CorrelationMismatch { .. }) => Ok(()),
            Some(err) => Err(format!("expected CorrelationMismatch, got {err:?}").into()),
            None => Err("expected CorrelationMismatch error, but none occurred".into()),
        }
    }

    /// Verify that all sent correlation IDs are unique.
    ///
    /// # Errors
    /// Returns an error if any correlation IDs are duplicated.
    pub fn verify_unique_correlation_ids(&self) -> TestResult {
        let mut sorted = self.sent_correlation_ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != self.sent_correlation_ids.len() {
            return Err("correlation IDs are not unique".into());
        }
        Ok(())
    }

    /// Verify that the response matches the expected message ID and payload.
    ///
    /// Uses the expected values stored when the envelope was configured via
    /// `set_envelope_with_payload`.
    ///
    /// # Errors
    /// Returns an error if the response is missing, expected values weren't set,
    /// or the response doesn't match.
    pub fn verify_response_matches_expected(&self) -> TestResult {
        let response = self.response.as_ref().ok_or("no response captured")?;
        let expected_id = self
            .expected_message_id
            .ok_or("expected message ID not set")?;
        let expected_payload = self
            .expected_payload
            .as_ref()
            .ok_or("expected payload not set")?;

        if response.id() != expected_id {
            return Err(format!("expected message ID {expected_id}, got {}", response.id()).into());
        }
        let response_payload = response.clone().into_parts().payload();
        if response_payload != expected_payload.as_bytes() {
            return Err(format!(
                "expected payload {:?}, got {:?}",
                expected_payload.as_bytes(),
                response_payload
            )
            .into());
        }
        Ok(())
    }

    /// Abort the server task.
    pub fn abort_server(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}

/// Run the frame processing loop for the echo server.
async fn run_frame_loop<T>(framed: &mut Framed<T, LengthDelimitedCodec>, mode: ServerMode)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(result) = framed.next().await {
        let Ok(bytes) = result else {
            warn!("client messaging server failed to decode frame");
            break;
        };

        let Some(response_bytes) = process_frame(mode, &bytes) else {
            warn!("client messaging server failed to process frame");
            break;
        };

        if framed.send(Bytes::from(response_bytes)).await.is_err() {
            break;
        }
    }
}
