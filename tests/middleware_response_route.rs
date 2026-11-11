//! End-to-end middleware behaviour on a response route.
//!
//! A response route installs its `Response` in a deferred slot on the
//! `ServiceResponse` rather than in the frame bytes. These scenarios drive a
//! prepared app over an in-memory transport to pin what middleware can and
//! cannot change: the request frame is mutable, the streamed body is not, and
//! returning a fresh response discards the deferred body.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{
    convert::Infallible,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use futures::StreamExt;
use rstest::rstest;
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    time::timeout,
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, Packet, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    middleware::{HandlerService, Next, Service, ServiceRequest, ServiceResponse, Transform},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult};

/// Route identifier used by the response-route scenarios.
const RESPONSE_ROUTE: u32 = 60;
/// Correlation identifier stamped on the response-route request.
const REQUEST_CORRELATION: u64 = 5;
/// Duplex capacity large enough for one request and its response.
const DUPLEX_CAPACITY: usize = 64;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Upper bound for a response that must arrive promptly.
const RESPONSE_SETTLE: Duration = Duration::from_secs(5);

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// How the response-route middleware treats the route it wraps.
enum MiddlewareMode {
    /// Forward to the route and edit the response frame.
    Forward,
    /// Answer without calling the route handler.
    ShortCircuit(Vec<u8>),
    /// Call the route handler, then replace its response with fresh bytes.
    Replace(Vec<u8>),
}

/// Middleware that edits a response route's request and response frame.
struct ResponseRouteMiddleware {
    /// Bytes the middleware forwarded to the route handler.
    forwarded: Arc<Mutex<Vec<u8>>>,
    /// How the middleware treats the wrapped route.
    mode: MiddlewareMode,
}

/// Service produced by [`ResponseRouteMiddleware`].
struct ResponseRouteService<S> {
    /// Wrapped route service.
    inner: S,
    /// Bytes the middleware forwarded to the route handler.
    forwarded: Arc<Mutex<Vec<u8>>>,
    /// How the middleware treats the wrapped route.
    mode: MiddlewareMode,
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for ResponseRouteMiddleware {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        HandlerService::from_service(
            service.id(),
            ResponseRouteService {
                inner: service,
                forwarded: Arc::clone(&self.forwarded),
                mode: match &self.mode {
                    MiddlewareMode::Forward => MiddlewareMode::Forward,
                    MiddlewareMode::ShortCircuit(bytes) => {
                        MiddlewareMode::ShortCircuit(bytes.clone())
                    }
                    MiddlewareMode::Replace(bytes) => MiddlewareMode::Replace(bytes.clone()),
                },
            },
        )
    }
}

#[async_trait]
impl<S> Service for ResponseRouteService<S>
where
    S: Service<Error = Infallible> + Send + Sync,
{
    type Error = Infallible;

    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        request.frame_mut().push(b'!');
        *self
            .forwarded
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = request.frame().to_vec();
        let correlation_id = request.correlation_id();
        if let MiddlewareMode::ShortCircuit(bytes) = &self.mode {
            return Ok(ServiceResponse::new(bytes.clone(), correlation_id));
        }
        let next = Next::new(&self.inner);
        let mut response = next.call(request).await?;
        if let MiddlewareMode::Replace(bytes) = &self.mode {
            return Ok(ServiceResponse::new(bytes.clone(), correlation_id));
        }
        response.frame_mut().push(b'?');
        Ok(response)
    }
}

/// Handler that answers with two frames and records the request it saw.
fn two_frame_handler(seen: Arc<Mutex<Vec<u8>>>) -> ResponseHandler<Envelope> {
    Arc::new(move |request: &Envelope| {
        let seen_for_handler = Arc::clone(&seen);
        Box::pin(async move {
            *seen_for_handler
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = request.payload_bytes().to_vec();
            let id = request.id();
            Response::Vec(vec![
                Envelope::new(id, None, vec![7]),
                Envelope::new(id, None, vec![8]),
            ])
        })
    })
}

/// Write one length-delimited frame to the duplex peer.
async fn write_frame(stream: &mut DuplexStream, payload: &[u8]) -> TestResult {
    let length = u32::try_from(payload.len())
        .map_err(|error| TestError::Msg(format!("frame too long: {error}")))?;
    let mut framed = BytesMut::with_capacity(4 + payload.len());
    framed.put_u32(length);
    framed.extend_from_slice(payload);
    stream.write_all(&framed).await?;
    Ok(())
}

/// Read `count` response frames and return their payloads in order.
async fn read_payloads<S>(frames: &mut S, count: usize) -> TestResult<Vec<Vec<u8>>>
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    let mut payloads = Vec::new();
    for _ in 0..count {
        let frame = frames
            .next()
            .await
            .transpose()
            .map_err(|error| TestError::Msg(format!("read response frame: {error}")))?
            .ok_or("the response ended early")?;
        let (envelope, consumed) = BincodeSerializer
            .deserialize::<Envelope>(&frame)
            .map_err(|error| TestError::Msg(format!("deserialize frame: {error}")))?;
        ensure(
            consumed == frame.len(),
            format!(
                "trailing bytes after envelope: consumed {consumed} of {}",
                frame.len()
            ),
        )?;
        payloads.push(envelope.payload_bytes().to_vec());
    }
    Ok(payloads)
}

/// Drive one request through a prepared app and return the response payloads.
async fn exchange(
    app: WireframeApp<BincodeSerializer, (), Envelope>,
    expected_frames: usize,
) -> TestResult<Vec<Vec<u8>>> {
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;
    let (mut client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
    let connection = tokio::spawn(async move { prepared.handle_connection_result(server).await });

    let request = Envelope::new(RESPONSE_ROUTE, Some(REQUEST_CORRELATION), vec![1, 2, 3]);
    let payload = BincodeSerializer
        .serialize(&request)
        .map_err(|error| TestError::Msg(format!("serialize request: {error}")))?;
    write_frame(&mut client, &payload).await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(client, codec.decoder());
    let payloads = timeout(RESPONSE_SETTLE, read_payloads(&mut framed, expected_frames))
        .await
        .map_err(|_| TestError::Msg("the response did not arrive".into()))??;

    drop(framed);
    connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?
        .map_err(|error| TestError::Msg(format!("connection failed: {error}")))?;
    Ok(payloads)
}

/// What one response-route exchange observed.
struct RouteObservation {
    /// Payloads the client received, in order.
    payloads: Vec<Vec<u8>>,
    /// Request frame the middleware forwarded to the route handler.
    forwarded: Vec<u8>,
    /// Request payload the route handler saw.
    seen: Vec<u8>,
}

/// Drive the two-frame route through `mode` and report what was observed.
async fn observe_route(
    mode: MiddlewareMode,
    expected_frames: usize,
) -> TestResult<RouteObservation> {
    let forwarded = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(RESPONSE_ROUTE, two_frame_handler(Arc::clone(&seen)))?
        .wrap(ResponseRouteMiddleware {
            forwarded: Arc::clone(&forwarded),
            mode,
        })?;

    let payloads = exchange(app, expected_frames).await?;
    let forwarded_request = forwarded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let handler_request = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(RouteObservation {
        payloads,
        forwarded: forwarded_request,
        seen: handler_request,
    })
}

/// Middleware edits a response route's request without rewriting its body.
///
/// The route's frames travel in the deferred slot, so the byte the middleware
/// appends to the response frame never reaches the client.
#[tokio::test]
async fn middleware_edits_a_response_route_without_rewriting_its_body() -> TestResult {
    let observation = observe_route(MiddlewareMode::Forward, 2).await?;

    ensure(
        observation.forwarded == [1, 2, 3, b'!'],
        "the middleware must forward the mutated request",
    )?;
    ensure(
        observation.seen == [1, 2, 3, b'!'],
        "the handler must observe the mutated request",
    )?;
    ensure(
        observation.payloads == vec![vec![7], vec![8]],
        format!(
            "the streamed body must reach the client unchanged: {:?}",
            observation.payloads
        ),
    )
}

/// A fresh middleware response hides the route's deferred body.
///
/// A short circuit answers without running the route handler; a replacement
/// runs the handler and then discards the frames it installed. Either way the
/// client receives only the middleware's own bytes.
#[rstest]
#[case::short_circuit(MiddlewareMode::ShortCircuit(vec![b'S']), b'S', false)]
#[case::replacement(MiddlewareMode::Replace(vec![b'R']), b'R', true)]
#[tokio::test]
async fn middleware_fresh_response_hides_the_deferred_body(
    #[case] mode: MiddlewareMode,
    #[case] replacement: u8,
    #[case] is_handler_reached: bool,
) -> TestResult {
    let observation = observe_route(mode, 1).await?;
    let was_handler_reached = !observation.seen.is_empty();

    ensure(
        was_handler_reached == is_handler_reached,
        "the route handler must run only when the middleware forwards to it",
    )?;
    ensure(
        observation.payloads == vec![vec![replacement]],
        format!(
            "only the middleware's bytes must reach the client: {:?}",
            observation.payloads
        ),
    )
}
