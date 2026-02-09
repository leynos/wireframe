//! Response forwarding helpers for frame handling.

use std::io;

use bytes::Bytes;
use futures::SinkExt;
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use crate::{
    app::{
        Envelope,
        Packet,
        PacketParts,
        combined_codec::ConnectionCodec,
        fragmentation_state::FragmentationState,
    },
    codec::FrameCodec,
    middleware::{HandlerService, Service, ServiceRequest},
    serializer::Serializer,
};

/// Forward a handler response, fragmenting if required, and write to the framed stream.
///
/// `forward_response` accepts an [`Envelope`], builds a [`ServiceRequest`], and
/// invokes `service.call(request)`. If the handler returns `Err(e)`, this is
/// treated as an application-level failure: the error is logged,
/// [`crate::metrics::inc_handler_errors()`] is incremented, and the function
/// returns `Ok(())` (intentional log-and-continue behaviour). Transport-level
/// I/O failures (for example during fragmentation, serialization, or frame
/// send) still return `io::Error` and are propagated to the caller.
pub(crate) async fn forward_response<S, E, W, F>(
    env: Envelope,
    service: &HandlerService<E>,
    ctx: super::ResponseContext<'_, S, W, F>,
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
                "handler error: id={id}, correlation_id={correlation_id:?}, error={e:?}",
                id = env.id,
                correlation_id = env.correlation_id
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
        let bytes = serialize_response(ctx.serializer, &response, env.id, correlation_id)?;
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
                Err(io::Error::other(err))
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
            Err(io::Error::other(e))
        }
    }
}

/// Send a response payload over the framed stream using codec-aware wrapping.
///
/// Wraps the raw payload bytes in the codec's native frame format via
/// [`FrameCodec::wrap_payload`] before writing to the underlying stream.
/// This ensures responses are encoded correctly for the configured protocol.
pub(super) async fn send_response_payload<F, W>(
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
        return Err(io::Error::other(e));
    }
    Ok(())
}
