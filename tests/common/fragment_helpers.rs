//! Shared helpers for fragment transport integration tests.
//!
//! Provides configuration builders, fragment encoding/decoding utilities,
//! and test application factories used across fragmentation test modules.

use std::{io, num::NonZeroUsize, time::Duration};

use futures::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::{
    sync::mpsc,
    time::{Duration as TokioDuration, timeout},
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Handler, Packet, WireframeApp},
    fragment::{
        FragmentationConfig,
        Fragmenter,
        Reassembler,
        ReassemblyError,
        decode_fragment_payload,
        encode_fragment_payload,
    },
    serializer::BincodeSerializer,
};

use super::TestResult;

/// Error type for fragment transport tests.
#[derive(Debug, Error)]
pub enum TestError {
    /// Test setup failed.
    #[error("test setup failed: {0}")]
    Setup(&'static str),
    /// Fragmentation operation failed.
    #[error("fragmentation failed: {0}")]
    Fragmentation(#[from] wireframe::fragment::FragmentationError),
    /// Encoding operation failed.
    #[error("encoding failed: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    /// Decoding operation failed.
    #[error("decoding failed: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    /// Reassembly operation failed.
    #[error("reassembly failed: {0}")]
    Reassembly(#[from] ReassemblyError),
    /// Send operation failed.
    #[error("send failed: {0}")]
    Send(String),
    /// Application error.
    #[error("application error: {0}")]
    App(#[from] wireframe::app::WireframeError),
    /// Other error.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
    /// Test assertion failed.
    #[error("assertion failed: {0}")]
    Assertion(String),
    /// IO operation failed.
    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),
    /// Operation timed out.
    #[error("timeout: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    /// Task join failed.
    #[error("task join failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl<T> From<mpsc::error::SendError<T>> for TestError {
    fn from(err: mpsc::error::SendError<T>) -> Self { TestError::Send(err.to_string()) }
}

/// Default route ID used in fragmentation tests.
pub const ROUTE_ID: u32 = 42;
/// Default correlation ID used in fragmentation tests.
pub const CORRELATION: Option<u64> = Some(7);

/// Create a fragmentation config for a given buffer capacity.
///
/// Uses a message limit of 16x capacity and a 30ms reassembly timeout.
///
/// # Errors
///
/// Returns an error if the message limit overflows or if the frame budget
/// is too small to accommodate fragment overhead.
pub fn fragmentation_config(capacity: usize) -> TestResult<FragmentationConfig> {
    let message_limit = capacity
        .checked_mul(16)
        .and_then(NonZeroUsize::new)
        .ok_or(TestError::Setup("message limit overflow or zero"))?;

    let config =
        FragmentationConfig::for_frame_budget(capacity, message_limit, Duration::from_millis(30))
            .ok_or(TestError::Setup(
            "frame budget must exceed fragment overhead",
        ))?;

    Ok(config)
}

/// Create a fragmentation config with a custom reassembly timeout.
///
/// # Errors
///
/// Returns an error if the underlying [`fragmentation_config`] fails.
pub fn fragmentation_config_with_timeout(
    capacity: usize,
    timeout_ms: u64,
) -> TestResult<FragmentationConfig> {
    let mut config = fragmentation_config(capacity)?;
    config.reassembly_timeout = Duration::from_millis(timeout_ms);
    Ok(config)
}

/// Fragment an envelope into multiple fragment envelopes.
///
/// Returns the original envelope wrapped in a vec if the payload fits in a
/// single fragment, otherwise returns the fragmented envelopes.
///
/// # Errors
///
/// Returns an error if fragmentation or fragment payload encoding fails.
pub fn fragment_envelope(env: &Envelope, fragmenter: &Fragmenter) -> TestResult<Vec<Envelope>> {
    let parts = env.clone().into_parts();
    let id = parts.id();
    let correlation = parts.correlation_id();
    let payload = parts.into_payload();

    if payload.len() <= fragmenter.max_fragment_size().get() {
        return Ok(vec![Envelope::new(id, correlation, payload)]);
    }

    let envelopes = fragmenter
        .fragment_bytes(payload)?
        .into_iter()
        .map(|fragment| {
            let (header, payload) = fragment.into_parts();
            encode_fragment_payload(header, &payload)
                .map(|encoded| Envelope::new(id, correlation, encoded))
                .map_err(TestError::from)
        })
        .collect::<Result<Vec<_>, TestError>>()?;

    Ok(envelopes)
}

/// Send a slice of envelopes over a framed client connection.
///
/// # Errors
///
/// Returns an error if serialization or sending fails.
pub async fn send_envelopes(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    envelopes: &[Envelope],
) -> TestResult {
    let serializer = BincodeSerializer;
    for env in envelopes {
        let bytes = serializer.serialize(env)?;
        client.send(bytes.into()).await?;
    }
    Ok(())
}

/// Read and reassemble a fragmented response from a client connection.
///
/// # Errors
///
/// Returns an error if reading, deserialization, or reassembly fails,
/// or if the stream ends before reassembly completes.
pub async fn read_reassembled_response(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    cfg: &FragmentationConfig,
) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);

    while let Some(frame) = client.next().await {
        let bytes = frame?;
        let (env, _) = serializer.deserialize::<Envelope>(&bytes)?;
        let payload = env.into_parts().into_payload();
        match decode_fragment_payload(&payload)? {
            Some((header, fragment)) => {
                if let Some(message) = reassembler.push(header, fragment)? {
                    return Ok(message.into_payload());
                }
            }
            None => return Ok(payload),
        }
    }

    Err(TestError::Setup("response stream ended before reassembly completed").into())
}

/// Create a handler that forwards received payloads to an unbounded channel.
///
/// # Panics
///
/// The returned handler panics if sending to the channel fails.
#[must_use]
pub fn make_handler(sender: &mpsc::UnboundedSender<Vec<u8>>) -> Handler<Envelope> {
    let tx = sender.clone();
    std::sync::Arc::new(move |env: &Envelope| {
        let tx = tx.clone();
        let payload = env.clone().into_parts().into_payload();
        Box::pin(async move {
            assert!(
                tx.send(payload).is_ok(),
                "handler channel send must succeed in tests"
            );
        })
    })
}

/// Create a test [`WireframeApp`] with fragmentation enabled.
///
/// # Errors
///
/// Returns an error if app creation or route registration fails.
pub fn make_app(
    capacity: usize,
    config: FragmentationConfig,
    sender: &mpsc::UnboundedSender<Vec<u8>>,
) -> TestResult<WireframeApp> {
    Ok(WireframeApp::new()?
        .buffer_capacity(capacity)
        .fragmentation(Some(config))
        .route(ROUTE_ID, make_handler(sender))?)
}

/// Spawn an app and return the client connection and server task handle.
pub fn spawn_app(
    app: WireframeApp,
) -> (
    Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let client = Framed::new(client_stream, codec.clone());
    let server = tokio::spawn(async move { app.handle_connection_result(server_stream).await });
    (client, server)
}

/// Build envelopes from a request, optionally fragmenting.
///
/// # Errors
///
/// Returns an error if fragmentation fails when `should_fragment` is true.
pub fn build_envelopes(
    request: Envelope,
    config: &FragmentationConfig,
    should_fragment: bool,
) -> TestResult<Vec<Envelope>> {
    if should_fragment {
        let fragmenter = Fragmenter::new(config.fragment_payload_cap);
        fragment_envelope(&request, &fragmenter)
    } else {
        Ok(vec![request])
    }
}

/// Assert that the handler received the expected payload.
///
/// # Errors
///
/// Returns an error if the receive times out, the channel is closed,
/// or the observed payload does not match the expected payload.
pub async fn assert_handler_observed(
    rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    expected: &[u8],
) -> TestResult<()> {
    let observed = timeout(TokioDuration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    if observed != expected {
        return Err(TestError::Assertion(format!(
            "observed payload mismatch: expected {expected:?}, got {observed:?}"
        ))
        .into());
    }
    Ok(())
}

/// Read and return the response payload with a 1-second timeout.
///
/// # Errors
///
/// Returns an error if the read times out or reassembly fails.
pub async fn read_response_payload(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    config: &FragmentationConfig,
) -> TestResult<Vec<u8>> {
    let response = timeout(
        TokioDuration::from_secs(1),
        read_reassembled_response(client, config),
    )
    .await??;
    Ok(response)
}
