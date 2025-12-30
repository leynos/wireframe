//! Shared helpers for frame decoding, reassembly, and response forwarding.
//!
//! Extracted from `connection.rs` to keep modules small and focused.

use std::{io, marker::PhantomData};

use futures::SinkExt;
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Encoder, Framed};

use super::{
    Envelope,
    Packet,
    PacketParts,
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

pub(crate) struct ResponseContext<'a, S, W, F, C>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    C: Encoder<F::Frame, Error = io::Error>,
{
    pub(crate) serializer: &'a S,
    pub(crate) framed: &'a mut Framed<W, C>,
    pub(crate) fragmentation: &'a mut Option<FragmentationState>,
    pub(crate) codec_marker: PhantomData<F>,
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
pub(crate) async fn forward_response<S, E, W, F, C>(
    env: Envelope,
    service: &HandlerService<E>,
    ctx: ResponseContext<'_, S, W, F, C>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    C: Encoder<F::Frame, Error = io::Error>,
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

        if send_response_payload::<F, W, C>(ctx.framed, bytes, env.id, correlation_id)
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

async fn send_response_payload<F, W, C>(
    framed: &mut Framed<W, C>,
    payload: Vec<u8>,
    id: u32,
    correlation_id: Option<u64>,
) -> io::Result<()>
where
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    C: Encoder<F::Frame, Error = io::Error>,
{
    let frame = F::wrap_payload(payload);
    if let Err(e) = framed.send(frame).await {
        warn!("failed to send response: id={id}, correlation_id={correlation_id:?}, error={e:?}");
        crate::metrics::inc_handler_errors();
        return Err(io::Error::other("send failed"));
    }
    Ok(())
}
