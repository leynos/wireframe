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
        *self.count += 1;
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

struct SendContext<'a, W, F>
where
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    codec: &'a F,
    framed: &'a mut Framed<W, ConnectionCodec<F>>,
    id: u32,
    correlation_id: Option<u64>,
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
                "handler error: id={}, correlation_id={:?}, error={e:?}",
                env.id, env.correlation_id
            );
            crate::metrics::inc_handler_errors();
            return Ok(());
        }
    };

    let parts = PacketParts::new(env.id, resp.correlation_id(), resp.into_inner())
        .inherit_correlation(env.correlation_id);
    let correlation_id = parts.correlation_id();
    let Ok(responses) = fragment_responses(ctx.fragmentation, parts, env.id, correlation_id) else {
        return Ok(()); // already logged
    };

    for response in responses {
        let Ok(bytes) = serialize_response(ctx.serializer, &response, env.id, correlation_id)
        else {
            break; // already logged
        };

        let send_ctx = SendContext {
            codec: ctx.codec,
            framed: ctx.framed,
            id: env.id,
            correlation_id,
        };
        if send_response_payload::<F, W>(send_ctx, Bytes::from(bytes))
            .await
            .is_err()
        {
            break;
        }
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
                    "failed to fragment response: id={id}, correlation_id={correlation_id:?}, \
                     error={err:?}"
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
                "failed to serialize response: id={id}, correlation_id={correlation_id:?}, \
                 error={e:?}"
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
async fn send_response_payload<F, W>(ctx: SendContext<'_, W, F>, payload: Bytes) -> io::Result<()>
where
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    let frame = ctx.codec.wrap_payload(payload);
    if let Err(e) = ctx.framed.send(frame).await {
        warn!(
            "failed to send response: id={}, correlation_id={:?}, error={e:?}",
            ctx.id, ctx.correlation_id
        );
        crate::metrics::inc_handler_errors();
        return Err(io::Error::other("send failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use bytes::{Buf, BufMut, Bytes, BytesMut};
    use futures::StreamExt;
    use tokio_util::codec::{Decoder, Encoder};

    use super::*;
    use crate::app::combined_codec::CombinedCodec;

    /// Test codec that wraps payloads with a distinctive tag byte.
    #[derive(Clone, Debug)]
    struct TestFrame {
        tag: u8,
        payload: Vec<u8>,
    }

    #[derive(Clone, Debug)]
    struct TestCodec {
        max_frame_length: usize,
        counter: Arc<AtomicUsize>,
    }

    impl TestCodec {
        fn new(max_frame_length: usize) -> Self {
            Self {
                max_frame_length,
                counter: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn wraps(&self) -> usize { self.counter.load(Ordering::SeqCst) }
    }

    #[derive(Clone, Debug)]
    struct TestAdapter {
        max_frame_length: usize,
    }

    impl TestAdapter {
        fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
    }

    impl Decoder for TestAdapter {
        type Item = TestFrame;
        type Error = io::Error;

        fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
            const HEADER_LEN: usize = 2;
            if src.len() < HEADER_LEN {
                return Ok(None);
            }

            let mut header = src.as_ref();
            let tag = header.get_u8();
            let payload_len = header.get_u8() as usize;
            if payload_len > self.max_frame_length {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "payload too large",
                ));
            }
            if src.len() < HEADER_LEN + payload_len {
                return Ok(None);
            }

            let mut frame_bytes = src.split_to(HEADER_LEN + payload_len);
            frame_bytes.advance(HEADER_LEN);
            let payload = frame_bytes.to_vec();

            Ok(Some(TestFrame { tag, payload }))
        }
    }

    impl Encoder<TestFrame> for TestAdapter {
        type Error = io::Error;

        fn encode(&mut self, item: TestFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
            if item.payload.len() > self.max_frame_length {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "payload too large",
                ));
            }

            let payload_len = u8::try_from(item.payload.len())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;
            dst.reserve(2 + item.payload.len());
            dst.put_u8(item.tag);
            dst.put_u8(payload_len);
            dst.extend_from_slice(&item.payload);
            Ok(())
        }
    }

    impl FrameCodec for TestCodec {
        type Frame = TestFrame;
        type Decoder = TestAdapter;
        type Encoder = TestAdapter;

        fn decoder(&self) -> Self::Decoder { TestAdapter::new(self.max_frame_length) }

        fn encoder(&self) -> Self::Encoder { TestAdapter::new(self.max_frame_length) }

        fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

        /// Wraps payload with tag byte 0x42 to verify codec-aware wrapping.
        fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
            self.counter.fetch_add(1, Ordering::SeqCst);
            TestFrame {
                tag: 0x42,
                payload: payload.to_vec(),
            }
        }

        fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.tag)) }

        fn max_frame_length(&self) -> usize { self.max_frame_length }
    }

    /// Verify `send_response_payload` uses `F::wrap_payload` to frame responses.
    #[tokio::test]
    async fn send_response_payload_wraps_with_codec() {
        let codec = TestCodec::new(64);
        let (client, server) = tokio::io::duplex(256);
        let combined = CombinedCodec::new(codec.decoder(), codec.encoder());
        let mut framed = Framed::new(server, combined);

        let payload = vec![1, 2, 3, 4];
        let send_ctx = SendContext {
            codec: &codec,
            framed: &mut framed,
            id: 1,
            correlation_id: Some(99),
        };
        send_response_payload::<TestCodec, _>(send_ctx, Bytes::from(payload.clone()))
            .await
            .expect("send should succeed");

        drop(framed);

        let combined_client = CombinedCodec::new(codec.decoder(), codec.encoder());
        let mut client_framed = Framed::new(client, combined_client);
        let frame = client_framed
            .next()
            .await
            .expect("expected a frame")
            .expect("decode should succeed");

        assert_eq!(frame.tag, 0x42, "wrap_payload should set tag to 0x42");
        assert_eq!(frame.payload, payload, "payload should match");
        assert_eq!(codec.wraps(), 1, "wrap_payload should advance codec state");
    }

    /// Verify `ResponseContext` fields are accessible and usable.
    #[tokio::test]
    async fn response_context_holds_references() {
        use crate::serializer::BincodeSerializer;

        let codec = TestCodec::new(64);
        let (_client, server) = tokio::io::duplex(256);
        let combined = CombinedCodec::new(codec.decoder(), codec.encoder());
        let mut framed = Framed::new(server, combined);
        let serializer = BincodeSerializer;
        let mut fragmentation: Option<FragmentationState> = None;

        let ctx: ResponseContext<'_, BincodeSerializer, _, TestCodec> = ResponseContext {
            serializer: &serializer,
            framed: &mut framed,
            fragmentation: &mut fragmentation,
            codec: &codec,
        };

        // Verify fields are accessible (compile-time check with runtime assertion)
        assert!(ctx.fragmentation.is_none());
    }

    /// Verify `send_response_payload` returns error on send failure.
    #[tokio::test]
    async fn send_response_payload_returns_error_on_failure() {
        let codec = TestCodec::new(4); // Small limit to trigger failure
        let (_client, server) = tokio::io::duplex(256);
        let combined = CombinedCodec::new(codec.decoder(), codec.encoder());
        let mut framed = Framed::new(server, combined);

        // Payload exceeds max_frame_length, so encode will fail
        let oversized_payload = vec![0u8; 100];
        let send_ctx = SendContext {
            codec: &codec,
            framed: &mut framed,
            id: 1,
            correlation_id: Some(99),
        };
        let result =
            send_response_payload::<TestCodec, _>(send_ctx, Bytes::from(oversized_payload)).await;

        assert!(
            result.is_err(),
            "expected send to fail for oversized payload"
        );
    }
}
