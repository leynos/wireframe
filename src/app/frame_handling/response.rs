//! Response forwarding helpers for frame handling.

use std::io;

use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    app::{Envelope, Packet, PacketParts, codec_driver::flush_pipeline_output},
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
/// fragmentation, protocol hooks (`before_send`), and outbound metrics before
/// serializing and sending via the codec.
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

    // Route through the pipeline: fragment → before_send → metrics
    ctx.pipeline.process(envelope)?;
    let mut output = ctx.pipeline.drain_output();
    flush_pipeline_output(ctx.serializer, ctx.codec, ctx.framed, &mut output).await
}
