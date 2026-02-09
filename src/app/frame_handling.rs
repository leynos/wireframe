//! Shared helpers for frame decoding, reassembly, and response forwarding.
//!
//! Extracted from `connection.rs` to keep modules small and focused.

use std::io;

use bytes::Bytes;
use futures::SinkExt;
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::{
    Envelope,
    Packet,
    PacketParts,
    combined_codec::ConnectionCodec,
    fragmentation_state::{FragmentProcessError, FragmentationState},
};
use crate::{
    codec::FrameCodec,
    middleware::{HandlerService, Service, ServiceRequest},
    serializer::Serializer,
};

/// Tracks consecutive deserialization failures and enforces a per-connection limit.
///
/// The counter increments on each failure; reaching `limit` terminates processing.
struct DeserFailureTracker<'a> {
    count: &'a mut u32,
    limit: u32,
}

impl<'a> DeserFailureTracker<'a> {
    fn new(count: &'a mut u32, limit: u32) -> Self { Self { count, limit } }

    fn record(
        &mut self,
        correlation_id: Option<u64>,
        context: &str,
        err: impl std::fmt::Debug,
    ) -> io::Result<()> {
        *self.count = self.count.saturating_add(1);
        warn!("{context}: correlation_id={correlation_id:?}, error={err:?}");
        crate::metrics::inc_deser_errors();
        if *self.count >= self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many deserialization failures",
            ));
        }
        Ok(())
    }
}

/// Context for writing handler responses to the framed stream.
///
/// Carries the serializer, codec, and mutable framing state for a connection.
pub(crate) struct ResponseContext<'a, S, W, F>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    pub(crate) serializer: &'a S,
    pub(crate) framed: &'a mut Framed<W, ConnectionCodec<F>>,
    pub(crate) fragmentation: &'a mut Option<FragmentationState>,
    pub(crate) codec: &'a F,
}

/// Attempt to reassemble a potentially fragmented envelope.
pub(crate) fn reassemble_if_needed(
    fragmentation: &mut Option<FragmentationState>,
    deser_failures: &mut u32,
    env: Envelope,
    max_deser_failures: u32,
) -> io::Result<Option<Envelope>> {
    let mut failures = DeserFailureTracker::new(deser_failures, max_deser_failures);

    if let Some(state) = fragmentation.as_mut() {
        let correlation_id = env.correlation_id;
        match state.reassemble(env) {
            Ok(Some(env)) => Ok(Some(env)),
            Ok(None) => Ok(None),
            Err(FragmentProcessError::Decode(err)) => {
                failures.record(correlation_id, "failed to decode fragment header", err)?;
                Ok(None)
            }
            Err(FragmentProcessError::Reassembly(err)) => {
                failures.record(correlation_id, "fragment reassembly failed", err)?;
                Ok(None)
            }
        }
    } else {
        Ok(Some(env))
    }
}

/// Forward a handler response, fragmenting if required, and write to the framed stream.
pub(crate) async fn forward_response<S, E, W, F>(
    env: Envelope,
    service: &HandlerService<E>,
    ctx: ResponseContext<'_, S, W, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    let request = ServiceRequest::new(env.payload, env.correlation_id);
    let resp = match service.call(request).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!(
                "handler error: id={id}, correlation_id={correlation_id:?}, error={error:?}",
                id = env.id,
                correlation_id = env.correlation_id,
                error = e
            );
            crate::metrics::inc_handler_errors();
            return Ok(());
        }
    };

    let parts = PacketParts::new(env.id, resp.correlation_id(), resp.into_inner())
        .inherit_correlation(env.correlation_id);
    let correlation_id = parts.correlation_id();
    let responses = fragment_responses(ctx.fragmentation, parts, env.id, correlation_id)?;

    for response in responses {
        let Ok(bytes) = serialize_response(ctx.serializer, &response, env.id, correlation_id)
        else {
            break; // already logged
        };

        send_response_payload::<F, W>(ctx.codec, ctx.framed, Bytes::from(bytes), &response).await?;
    }

    Ok(())
}

fn fragment_responses(
    fragmentation: &mut Option<FragmentationState>,
    parts: PacketParts,
    id: u32,
    correlation_id: Option<u64>,
) -> io::Result<Vec<Envelope>> {
    let envelope = Envelope::from_parts(parts);
    match fragmentation.as_mut() {
        Some(state) => match state.fragment(envelope) {
            Ok(fragmented) => Ok(fragmented),
            Err(err) => {
                warn!(
                    concat!(
                        "failed to fragment response: id={id}, correlation_id={correlation_id:?}, ",
                        "error={err:?}"
                    ),
                    id = id,
                    correlation_id = correlation_id,
                    err = err
                );
                crate::metrics::inc_handler_errors();
                Err(io::Error::other("fragmentation failed"))
            }
        },
        None => Ok(vec![envelope]),
    }
}

fn serialize_response<S: Serializer>(
    serializer: &S,
    response: &Envelope,
    id: u32,
    correlation_id: Option<u64>,
) -> io::Result<Vec<u8>> {
    match serializer.serialize(response) {
        Ok(bytes) => Ok(bytes),
        Err(e) => {
            warn!(
                concat!(
                    "failed to serialize response: id={id}, correlation_id={correlation_id:?}, ",
                    "error={e:?}"
                ),
                id = id,
                correlation_id = correlation_id,
                e = e
            );
            crate::metrics::inc_handler_errors();
            Err(io::Error::other("serialization failed"))
        }
    }
}

/// Send a response payload over the framed stream using codec-aware wrapping.
///
/// Wraps the raw payload bytes in the codec's native frame format via
/// [`FrameCodec::wrap_payload`] before writing to the underlying stream.
/// This ensures responses are encoded correctly for the configured protocol.
async fn send_response_payload<F, W>(
    codec: &F,
    framed: &mut Framed<W, ConnectionCodec<F>>,
    payload: Bytes,
    response: &Envelope,
) -> io::Result<()>
where
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    let frame = codec.wrap_payload(payload);
    if let Err(e) = framed.send(frame).await {
        let id = response.id;
        let correlation_id = response.correlation_id;
        warn!("failed to send response: id={id}, correlation_id={correlation_id:?}, error={e:?}");
        crate::metrics::inc_handler_errors();
        return Err(io::Error::other("send failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::StreamExt;
    use rstest::{fixture, rstest};
    use tokio::io::DuplexStream;

    use super::*;
    use crate::{
        app::combined_codec::CombinedCodec,
        test_helpers::{TestAdapter, TestCodec},
    };

    struct FramedHarness {
        codec: TestCodec,
        server_framed: Framed<DuplexStream, CombinedCodec<TestAdapter, TestAdapter>>,
        client_framed: Framed<DuplexStream, CombinedCodec<TestAdapter, TestAdapter>>,
    }

    fn build_harness(max_frame_length: usize) -> FramedHarness {
        let codec = TestCodec::new(max_frame_length);
        let (client, server) = tokio::io::duplex(256);
        let server_codec = CombinedCodec::new(codec.decoder(), codec.encoder());
        let client_codec = CombinedCodec::new(codec.decoder(), codec.encoder());
        let server_framed = Framed::new(server, server_codec);
        let client_framed = Framed::new(client, client_codec);

        FramedHarness {
            codec,
            server_framed,
            client_framed,
        }
    }

    #[fixture]
    fn default_harness() -> FramedHarness {
        let harness = build_harness(64);
        assert_eq!(harness.codec.max_frame_length(), 64);
        harness
    }

    #[fixture]
    fn small_harness() -> FramedHarness {
        let harness = build_harness(4);
        assert_eq!(harness.codec.max_frame_length(), 4);
        harness
    }

    /// Verify `send_response_payload` uses `F::wrap_payload` to frame responses.
    #[rstest]
    #[tokio::test]
    async fn send_response_payload_wraps_with_codec(mut default_harness: FramedHarness) {
        let payload = vec![1, 2, 3, 4];
        let response = Envelope::new(1, Some(99), payload.clone());
        send_response_payload::<TestCodec, _>(
            &default_harness.codec,
            &mut default_harness.server_framed,
            Bytes::from(payload.clone()),
            &response,
        )
        .await
        .expect("send should succeed");

        let frame = default_harness
            .client_framed
            .next()
            .await
            .expect("expected a frame")
            .expect("decode should succeed");

        assert_eq!(frame.tag, 0x42, "wrap_payload should set tag to 0x42");
        assert_eq!(frame.payload, payload, "payload should match");
        assert_eq!(
            default_harness.codec.wraps(),
            1,
            "wrap_payload should advance codec state"
        );
    }

    /// Verify `ResponseContext` fields are accessible and usable.
    #[rstest]
    #[tokio::test]
    async fn response_context_holds_references(mut default_harness: FramedHarness) {
        use crate::serializer::BincodeSerializer;

        let serializer = BincodeSerializer;
        let mut fragmentation: Option<FragmentationState> = None;

        let ctx: ResponseContext<'_, BincodeSerializer, _, TestCodec> = ResponseContext {
            serializer: &serializer,
            framed: &mut default_harness.server_framed,
            fragmentation: &mut fragmentation,
            codec: &default_harness.codec,
        };

        // Verify fields are accessible (compile-time check with runtime assertion)
        assert!(ctx.fragmentation.is_none());
    }

    /// Verify `send_response_payload` returns error on send failure.
    #[rstest]
    #[tokio::test]
    async fn send_response_payload_returns_error_on_failure(mut small_harness: FramedHarness) {
        // Payload exceeds max_frame_length, so encode will fail
        let oversized_payload = vec![0u8; 100];
        let response = Envelope::new(1, Some(99), oversized_payload.clone());
        let result = send_response_payload::<TestCodec, _>(
            &small_harness.codec,
            &mut small_harness.server_framed,
            Bytes::from(oversized_payload),
            &response,
        )
        .await;

        assert!(
            result.is_err(),
            "expected send to fail for oversized payload"
        );
    }
}
