//! Response forwarding helpers for frame handling.

use std::io;

use futures::{Sink, StreamExt};
use log::warn;

use crate::{
    app::{
        Envelope,
        Packet,
        PacketParts,
        codec_driver::{ActorFrameSink, send_native_frame},
    },
    codec::FrameCodec,
    connection::ResponseOutcome,
    hooks::ConnectionContext,
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
pub(crate) async fn forward_response<S, E, K, F>(
    env: Envelope,
    service: &HandlerService<E>,
    ctx: super::ResponseContext<'_, S, K, F>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
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

    let is_streaming = matches!(response, Response::Stream(_) | Response::MultiPacket(_));
    let outcome = {
        let mut sink = ActorFrameSink::new(
            ctx.serializer,
            ctx.codec,
            &mut *ctx.sink,
            &mut *ctx.pipeline,
        );
        ctx.actor
            .install_response(response, correlation_id, &mut sink)
            .await
            .map_err(install_error)?
    };

    let Some(protocol) = ctx.protocol else {
        return Ok(());
    };
    let mut context = ConnectionContext;
    // The protocol terminator is a complete codec frame. Writing it unchanged
    // preserves metadata a custom codec adds outside the payload, and running
    // it through the sink's `before_send` hook matches application frames.
    if owes_terminator(outcome, is_streaming)
        && let Some(frame) = protocol.stream_end_frame(&mut context)
    {
        send_native_frame::<K, F>(&mut *ctx.sink, &mut *ctx.pipeline, frame).await?;
    }
    // A cancelled response belongs to a closing connection: its completion
    // hook must not run, matching the actor's own shutdown behaviour.
    if outcome != ResponseOutcome::Cancelled {
        protocol.on_command_end(&mut context);
    }
    Ok(())
}

/// Whether a completed streamed response is owed a protocol terminator.
///
/// Only a stream or multi-packet response that ran to completion ends with a
/// terminator; a failed or cancelled one must not emit another frame.
const fn owes_terminator(outcome: ResponseOutcome, is_streaming: bool) -> bool {
    match outcome {
        ResponseOutcome::Completed => is_streaming,
        ResponseOutcome::Failed | ResponseOutcome::Cancelled => false,
    }
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
