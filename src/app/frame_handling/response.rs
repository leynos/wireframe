//! Response forwarding helpers for frame handling.

use std::io;

use futures::StreamExt;
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    app::{Envelope, Packet, PacketParts, codec_driver::ActorFrameSink},
    codec::FrameCodec,
    message::EncodeWith,
    middleware::{HandlerService, Service, ServiceRequest},
    response::{FrameStream, Response, WireframeError},
    serializer::Serializer,
};

/// Forward a handler response through the connection actor to the transport.
///
/// `forward_response` accepts an [`Envelope`], builds a [`ServiceRequest`], and
/// invokes `service.call(request)`. If the handler returns `Err(e)`, this is
/// treated as an application-level failure: the error is logged,
/// [`crate::metrics::inc_handler_errors()`] is incremented, and the function
/// returns `Ok(())` (intentional log-and-continue behaviour). Transport-level
/// I/O failures (for example during fragmentation, serialization, or frame
/// send) still return `io::Error` and are propagated to the caller.
///
/// A route registered with `route_response` stores its explicit
/// [`Response`] in the [`ServiceResponse`] deferred-body slot. That body is
/// installed on the connection actor with the request's correlation
/// identifier: `Single`, `Vec`, and `Empty` complete immediately, while
/// `Stream` and `MultiPacket` are driven to completion before the next inbound
/// frame is read. Stream items inherit the request's correlation identifier
/// lazily as they are polled; an item that supplies a different identifier is
/// overwritten and logged as a warning. A route registered with `route` has no
/// deferred body, so its bytes are wrapped in one [`Envelope`] and installed as
/// `Response::Single`, which is exactly one reply without a stream terminator.
///
/// Responses pass through the [`FramePipeline`] inside the actor sink, which
/// applies fragmentation, outbound metrics, serialization, codec wrapping, and
/// the protocol `before_send` hook in that order. The pipeline owns
/// per-connection hooks, so `before_send` observes the same
/// [`crate::hooks::ConnectionContext`] for every frame on the connection.
///
/// [`ServiceResponse`]: crate::middleware::ServiceResponse
/// [`FramePipeline`]: crate::app::codec_driver::FramePipeline
pub(crate) async fn forward_response<S, E, W, F>(
    env: Envelope,
    service: &HandlerService<E>,
    ctx: super::ResponseContext<'_, S, W, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let request_id = env.id;
    let correlation_id = env.correlation_id;
    let request = ServiceRequest::new(env.payload, correlation_id);
    let resp = match service.call(request).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!("handler error: id={request_id}, correlation_id={correlation_id:?}, error={e:?}");
            crate::metrics::inc_handler_errors();
            return Ok(());
        }
    };

    let response = resp.take_deferred().map_or_else(
        || {
            let parts = PacketParts::new(request_id, resp.correlation_id(), resp.into_inner())
                .inherit_correlation(correlation_id);
            Response::Single(Envelope::from_parts(parts))
        },
        |response| inherit_stream_correlation(response, correlation_id),
    );

    let mut sink = ActorFrameSink::new(
        ctx.serializer,
        ctx.codec,
        &mut *ctx.framed,
        &mut *ctx.pipeline,
    );
    ctx.actor
        .install_response(response, correlation_id, &mut sink)
        .await
        .map_err(install_error)
}

/// Stamp the request correlation identifier onto lazily polled stream items.
fn inherit_stream_correlation(
    response: Response<Envelope, ()>,
    correlation_id: Option<u64>,
) -> Response<Envelope, ()> {
    match response {
        Response::Stream(stream) => Response::Stream(inherit_on_stream(stream, correlation_id)),
        other => other,
    }
}

/// Map a frame stream so each item inherits the request correlation.
fn inherit_on_stream<E>(
    stream: FrameStream<Envelope, E>,
    correlation_id: Option<u64>,
) -> FrameStream<Envelope, E>
where
    E: Send + 'static,
{
    stream
        .map(move |item| {
            item.map(|envelope| {
                let parts = envelope.into_parts().inherit_correlation(correlation_id);
                Envelope::from_parts(parts)
            })
        })
        .boxed()
}

/// Convert an actor installation failure into the inbound loop's I/O error.
fn install_error(failure: WireframeError<()>) -> io::Error {
    match failure {
        WireframeError::Io(error) => error,
        other => io::Error::other(format!("failed to install handler response: {other:?}")),
    }
}
