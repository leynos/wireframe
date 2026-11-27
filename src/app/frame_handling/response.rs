//! Response forwarding helpers for frame handling.
//!
//! [`forward_response`] runs a buffered route's middleware service for one
//! request envelope and installs the resulting [`Response`] on the connection
//! actor, and [`forward_service_response`] does the same for a streaming
//! handler's optional reply. [`Dispatch`] reports whether the loop may serve
//! another inbound frame or must stop after a failed or cancelled stream.
//!
//! Responses travel through the actor's `FramePipeline`, which applies
//! fragmentation, outbound metrics, serialization, codec wrapping, and the
//! protocol `before_send` hook. The streaming dispatch modules reuse this path
//! for every reply their handlers produce.

use std::io;

use bytes::Bytes;
use futures::{Sink, StreamExt};
use log::warn;
use tracing::Instrument;

use super::{CompletionSequence, response_pushes::run_handler_with_pushes};
use crate::{
    app::{
        Envelope,
        Packet,
        PacketParts,
        codec_driver::{ActorFrameSink, send_native_frame},
    },
    codec::FrameCodec,
    connection::{ResponseOutcome, TerminatorCorrelation, WriteOutcome},
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
/// frame is dispatched; the read half may buffer one request meanwhile. Stream
/// items inherit the request's correlation identifier lazily as they are
/// polled; an item that supplies a different identifier is overwritten and
/// logged as a warning. A route registered with `route` has no deferred body,
/// so its bytes are wrapped in one [`Envelope`] and installed as
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
    let Some(handled) = run_handler_with_pushes(&mut ctx, service.call(request)).await? else {
        // Shutdown abandoned the handler while one of its pushes blocked. No
        // response was delivered, so the connection stops here.
        return Ok(Dispatch::Stop);
    };
    let resp = match handled {
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

/// Forward the reply produced by a streaming route handler.
///
/// A streaming handler receives [`RequestParts`](crate::request::RequestParts)
/// rather than an [`Envelope`], so the reply is rebuilt from the original route
/// identifier and correlation identifier. An absent reply means the handler
/// chose not to answer and nothing is written.
///
/// The reply is installed on the connection actor as one [`Response::Single`],
/// so it passes through the same pipeline, fragmentation, and outbound hooks as
/// a buffered route's response. It completes immediately and therefore owes no
/// stream terminator.
///
/// A silent handler still completes the request/response cycle, so it runs the
/// completion hook exactly as a buffered [`Response::Empty`] does. Nothing was
/// written for it, so no terminator and no delivery span apply.
pub(crate) async fn forward_service_response<S, K, F>(
    route_id: u32,
    correlation_id: Option<u64>,
    reply: Option<Bytes>,
    mut ctx: super::ResponseContext<'_, S, K, F>,
) -> io::Result<Dispatch>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let Some(payload) = reply else {
        // The pipeline owns the connection's hook owner, so a silent handler
        // still runs the completion hook exactly as a buffered empty reply
        // does.
        ctx.pipeline.on_command_end();
        return Ok(Dispatch::Continue);
    };
    // The envelope reply path still carries `Vec<u8>` pending the ADR 009
    // migration, so the handler's `Bytes` is converted once at this boundary.
    let envelope =
        Envelope::from_parts(PacketParts::new(route_id, correlation_id, payload.to_vec()));
    let delivery = DeliverySpan {
        request_id: route_id,
        correlation_id,
        is_streaming: false,
    };
    let outcome = deliver(&mut ctx, delivery, Response::Single(envelope)).await?;
    Ok(dispatch_after(outcome))
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
    complete_response(ctx, delivery, outcome).await
}

/// Run the completion sequence a delivered response owes.
///
/// [`CompletionSequence`] names the order — error, terminator, completion —
/// and this unit executes it against the pipeline that owns the connection's
/// callbacks. A terminator abandoned by shutdown suppresses the completion
/// hook, because the connection is closing and half a response must not
/// report completion.
async fn complete_response<S, K, F>(
    ctx: &mut super::ResponseContext<'_, S, K, F>,
    delivery: &DeliverySpan,
    outcome: ResponseOutcome,
) -> io::Result<ResponseOutcome>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let sequence = CompletionSequence::for_outcome(outcome, delivery.is_streaming);
    // A failed stream stopped on a protocol error. Reporting it before the
    // completion hook keeps both callbacks on the connection's single hook
    // owner and context; the actor cannot dispatch it because it deliberately
    // holds no protocol reference of its own.
    if sequence.reports_error() {
        ctx.pipeline.handle_error();
    }
    // The pipeline owns the connection's context, so every callback below sees
    // the same connection identifier as `before_send`.
    if sequence.writes_terminator() {
        // The protocol terminator is a complete codec frame. Writing it
        // unchanged preserves metadata a custom codec adds outside the payload,
        // and running it through the sink's `before_send` hook matches
        // application frames. The pipeline's construction selected the
        // preserving policy for this path.
        debug_assert_eq!(
            ctx.pipeline.terminator_correlation(),
            TerminatorCorrelation::PREPARED,
            "a prepared connection never restamps a codec-frame terminator"
        );
        let abandoned = match ctx.pipeline.stream_end_frame() {
            Some(frame) => !write_terminator::<S, K, F>(ctx, frame).await?,
            None => false,
        };
        if abandoned {
            return Ok(ResponseOutcome::Cancelled);
        }
    }
    // A cancelled response belongs to a closing connection: its completion
    // hook must not run, matching the actor's own shutdown behaviour.
    if sequence.completes() {
        ctx.pipeline.on_command_end();
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
    mut frame: F::Frame,
) -> io::Result<bool>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    // A prepared connection's terminator is a complete codec frame, so its
    // pipeline's preserving policy applies: it is written unchanged and never
    // restamped.
    ctx.pipeline
        .terminator_correlation()
        .apply_codec_frame(&mut frame);
    let writing = send_native_frame::<K, F>(&mut *ctx.sink, &mut *ctx.pipeline, frame);
    // A terminator is one physical frame written straight to the transport, so
    // it has no finer progress to report than the write itself completing.
    let outcome = ctx.actor.write_frame_bounded(writing, None).await?;
    Ok(outcome == WriteOutcome::Written)
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
