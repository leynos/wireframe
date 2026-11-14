//! Response forwarding helpers for frame handling.

use std::io;

use futures::{Sink, StreamExt};
use log::warn;
use tracing::Instrument;

use crate::{
    app::{
        Envelope,
        Packet,
        PacketParts,
        codec_driver::{ActorFrameSink, send_native_frame},
    },
    codec::FrameCodec,
    connection::{ResponseOutcome, WriteOutcome},
    hooks::ConnectionContext,
    message::EncodeWith,
    middleware::{HandlerService, Service, ServiceRequest},
    response::{FrameStream, Response, WireframeError},
    serializer::Serializer,
};

/// Whether the connection loop may dispatch another inbound frame.
///
/// A failed or cancelled response leaves its stream without a terminator, so a
/// client cannot tell where that stream ended. Emitting another response
/// afterwards would be read as part of the truncated stream, so the connection
/// stops instead of continuing with the buffered request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Dispatch {
    /// The response finished; the next inbound frame may be served.
    Continue,
    /// The response failed or was cancelled; the connection must stop writing.
    Stop,
}

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
/// A cancelled response is the last thing written on its connection: its
/// stream has no terminator, so the returned [`Dispatch::Stop`] tells the
/// inbound loop not to serve the request buffered behind it.
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
    mut ctx: super::ResponseContext<'_, S, K, F>,
) -> io::Result<Dispatch>
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
            return Ok(Dispatch::Continue);
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

    let delivery = DeliverySpan {
        request_id,
        correlation_id,
        is_streaming: matches!(response, Response::Stream(_) | Response::MultiPacket(_)),
    };
    let outcome = deliver(&mut ctx, delivery, response).await?;
    Ok(dispatch_after(outcome))
}

/// Drive one response to its terminator, completion hook, and final outcome.
///
/// The `response_delivery` span covers the whole exchange, so the terminator
/// write and the completion hook it precedes belong to the same span as the
/// frames they follow.
async fn deliver<S, K, F>(
    ctx: &mut super::ResponseContext<'_, S, K, F>,
    delivery: DeliverySpan,
    response: Response<Envelope, ()>,
) -> io::Result<ResponseOutcome>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let span = tracing::debug_span!(
        "response_delivery",
        request_id = delivery.request_id,
        correlation_id = ?delivery.correlation_id,
        streaming = delivery.is_streaming,
        outcome = tracing::field::Empty,
    );
    let result = write_response(ctx, &delivery, response)
        .instrument(span.clone())
        .await;
    result.as_ref().map_or_else(
        |_| span.record("outcome", "transport_error"),
        |outcome| span.record("outcome", tracing::field::debug(outcome)),
    );
    result
}

/// Write `response`, then the protocol terminator and completion hook it owes.
async fn write_response<S, K, F>(
    ctx: &mut super::ResponseContext<'_, S, K, F>,
    delivery: &DeliverySpan,
    response: Response<Envelope, ()>,
) -> io::Result<ResponseOutcome>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let outcome = drive_through_actor(ctx, delivery, response).await?;

    let Some(protocol) = ctx.protocol else {
        return Ok(outcome);
    };
    let mut context = ConnectionContext;
    if owes_terminator(outcome, delivery.is_streaming) {
        // The protocol terminator is a complete codec frame. Writing it
        // unchanged preserves metadata a custom codec adds outside the payload,
        // and running it through the sink's `before_send` hook matches
        // application frames.
        let abandoned = match protocol.stream_end_frame(&mut context) {
            Some(frame) => !write_terminator::<S, K, F>(ctx, frame).await?,
            None => false,
        };
        if abandoned {
            // Shutdown abandoned the terminator, so the connection is closing
            // and the completion hook must not run for a half-written response.
            return Ok(ResponseOutcome::Cancelled);
        }
    }
    // A cancelled response belongs to a closing connection: its completion
    // hook must not run, matching the actor's own shutdown behaviour.
    if outcome != ResponseOutcome::Cancelled {
        protocol.on_command_end(&mut context);
    }
    Ok(outcome)
}

/// Whether the connection can serve another frame after `outcome`.
///
/// Only a completed response ends with a terminator. A failed stream reports a
/// protocol error, which the actor records without returning an I/O error, and
/// a cancelled one is abandoned; neither writes a terminator, so a client
/// cannot tell where that stream ended. Serving the buffered request would
/// make its response read as part of the truncated stream, so both stop
/// dispatch.
const fn dispatch_after(outcome: ResponseOutcome) -> Dispatch {
    match outcome {
        ResponseOutcome::Completed => Dispatch::Continue,
        ResponseOutcome::Failed | ResponseOutcome::Cancelled => Dispatch::Stop,
    }
}

/// Bounded identifiers recorded for one response delivery.
struct DeliverySpan {
    /// Message identifier the response answers.
    request_id: u32,
    /// Correlation identifier stamped on the response, when known.
    correlation_id: Option<u64>,
    /// Whether the response is a stream or multi-packet body.
    is_streaming: bool,
}

/// Install `response` on the actor and drive it through the connection sink.
async fn drive_through_actor<S, K, F>(
    ctx: &mut super::ResponseContext<'_, S, K, F>,
    delivery: &DeliverySpan,
    response: Response<Envelope, ()>,
) -> io::Result<ResponseOutcome>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let mut sink = ActorFrameSink::new(
        ctx.serializer,
        ctx.codec,
        &mut *ctx.sink,
        &mut *ctx.pipeline,
    );
    ctx.actor
        .install_response(response, delivery.correlation_id, &mut sink)
        .await
        .map_err(install_error)
}

/// Write the protocol terminator under the actor's write bounds.
///
/// Returns `true` when the frame reached the sink. The terminator follows a
/// response the peer was reading, but the peer may stop reading before this
/// last frame, and an unbounded write would then pin a graceful drain after the
/// response itself completed. The frame is therefore written through the same
/// bound as every buffered frame: a stalled write is abandoned at shutdown or
/// once the peer's idle grace expires, and an abandoned terminator suppresses
/// the completion hook.
async fn write_terminator<S, K, F>(
    ctx: &mut super::ResponseContext<'_, S, K, F>,
    frame: F::Frame,
) -> io::Result<bool>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let writing = send_native_frame::<K, F>(&mut *ctx.sink, &mut *ctx.pipeline, frame);
    // A terminator is one physical frame written straight to the transport, so
    // it has no finer progress to report than the write itself completing.
    let outcome = ctx.actor.write_frame_bounded(writing, None).await?;
    Ok(outcome == WriteOutcome::Written)
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
