//! Response forwarding helpers for frame handling.

use std::io;

use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    app::{
        Envelope,
        Packet,
        PacketParts,
        codec_driver::{OutboundTransport, flush_pipeline_output},
    },
    codec::FrameCodec,
    message::EncodeWith,
    middleware::{HandlerService, Service, ServiceRequest},
    serializer::Serializer,
};

/// Forward a handler response through the pipeline and write to the framed stream.
///
/// `forward_response` accepts an [`Envelope`], builds a [`ServiceRequest`], and
/// invokes `service.call(request)`. If the handler returns `Err(e)`, this is
/// treated as an application-level failure: the error is logged,
/// [`crate::metrics::inc_handler_errors()`] is incremented, and the function
/// returns `Ok(())` (intentional log-and-continue behaviour). Transport-level
/// I/O failures (for example during fragmentation, serialization, or frame
/// send) still return `io::Error` and are propagated to the caller.
///
/// Responses are processed through the [`FramePipeline`] which applies
/// fragmentation and outbound metrics while buffering envelopes. The flush
/// stage then serializes each envelope, wraps it via the codec, runs the
/// configured protocol `before_send` hook, and writes it to the transport.
/// The pipeline owns per-connection hooks, so `before_send` observes the same
/// [`crate::hooks::ConnectionContext`] for every frame on the connection.
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
    Envelope: EncodeWith<S>,
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
    let envelope = Envelope::from_parts(parts);

    write_envelope(envelope, ctx).await
}

/// Forward the reply produced by a streaming route handler.
///
/// A streaming handler receives [`RequestParts`](crate::request::RequestParts)
/// rather than an [`Envelope`], so the reply is rebuilt from the original route
/// identifier and correlation identifier. An empty `payload` means the handler
/// chose not to reply and nothing is written.
///
/// Replies use the same pipeline, fragmentation, and outbound hooks as a
/// buffered route, so `before_send` observes one connection context regardless
/// of which dispatch path produced the frame.
pub(crate) async fn forward_service_response<S, W, F>(
    route_id: u32,
    correlation_id: Option<u64>,
    payload: Option<Vec<u8>>,
    ctx: super::ResponseContext<'_, S, W, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let Some(payload) = payload else {
        return Ok(());
    };
    let envelope = Envelope::from_parts(PacketParts::new(route_id, correlation_id, payload));
    write_envelope(envelope, ctx).await
}

/// Send one envelope through the outbound pipeline and transport.
async fn write_envelope<S, W, F>(
    envelope: Envelope,
    ctx: super::ResponseContext<'_, S, W, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    // Route through the pipeline: fragment → metrics, then flush with
    // serialize → wrap_payload → before_send → send.
    ctx.pipeline.process(envelope)?;
    let mut transport = OutboundTransport {
        serializer: ctx.serializer,
        codec: ctx.codec,
        framed: &mut *ctx.framed,
    };
    flush_pipeline_output(&mut transport, ctx.pipeline).await
}
