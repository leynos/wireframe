//! Ping/pong example exchanging typed request and response packets.
//!
//! Demonstrates custom packet structs and middleware that maps `Ping` to
//! `Pong` responses.

use std::{net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use tracing::{error, info};
use wireframe::{
    app::{Envelope, Packet, Result as AppResult},
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
};

#[path = "support/runtime_bootstrap.rs"]
mod runtime_bootstrap;
#[path = "support/server_loop.rs"]
mod server_loop;

/// Application type assembled from bincode envelopes and the middleware chain.
type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
/// Request payload whose counter is incremented by the pong service.
struct Ping(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
/// Successful response payload carrying the incremented counter.
struct Pong(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
/// Error response payload used when decoding, encoding, or incrementing fails.
struct ErrorMsg(String);

/// Encodes a protocol error while retaining an empty payload fallback for the
/// example's infallible middleware boundary.
fn encode_error(msg: impl Into<String>) -> Vec<u8> {
    let err = ErrorMsg(msg.into());
    match err.to_bytes() {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(error = ?e, "failed to encode error");
            Vec::new()
        }
    }
}

/// Route ID registered for ping requests in the sample application.
const PING_ID: u32 = 1;

/// Handler invoked for `PING_ID` messages.
///
/// The middleware chain generates the actual response, so this
/// handler intentionally performs no work.
#[expect(
    clippy::unused_async,
    reason = "Keep async signature to match Handler and Transform trait expectations"
)]
async fn ping_handler() {}

/// Middleware adapter that turns a decoded ping into a pong response.
struct PongMiddleware;

/// Service wrapper that performs ping decoding, incrementing, and encoding
/// while preserving the inner handler's correlation ID.
struct PongService<S> {
    /// Inner handler retained so middleware can compose with normal routing.
    inner: S,
}

#[async_trait]
impl<S> Service for PongService<S>
where
    S: Service<Error = std::convert::Infallible> + Send + Sync + 'static,
{
    type Error = std::convert::Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let cid = req.correlation_id();
        let (ping_req, _) = match Ping::from_bytes(req.frame()) {
            Ok(val) => val,
            Err(e) => {
                error!(error = ?e, "failed to decode ping");
                return Ok(ServiceResponse::new(
                    encode_error(format!("decode error: {e:?}")),
                    cid,
                ));
            }
        };
        let mut response = self.inner.call(req).await?;
        let pong_resp = if let Some(v) = ping_req.0.checked_add(1) {
            Pong(v)
        } else {
            error!(value = ping_req.0, "ping overflowed");
            return Ok(ServiceResponse::new(encode_error("overflow"), cid));
        };
        match pong_resp.to_bytes() {
            Ok(bytes) => *response.frame_mut() = bytes,
            Err(e) => {
                error!(error = ?e, "failed to encode pong");
                return Ok(ServiceResponse::new(
                    encode_error(format!("encode error: {e:?}")),
                    cid,
                ));
            }
        }
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for PongMiddleware {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(id, PongService { inner: service })
    }
}

/// Middleware marker that adds request/response tracing around a handler.
struct Logging;

/// Service wrapper that logs both sides of a call before returning its result.
struct LoggingService<S> {
    /// Inner service whose result is observed without changing its semantics.
    inner: S,
}

#[async_trait]
impl<S> Service for LoggingService<S>
where
    S: Service<Error = std::convert::Infallible> + Send + Sync + 'static,
{
    type Error = std::convert::Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        info!(frame = ?req.frame(), "request");
        let resp = self.inner.call(req).await?;
        info!(frame = ?resp.frame(), "response");
        Ok(resp)
    }
}

#[async_trait]
impl<E: Packet> Transform<HandlerService<E>> for Logging {
    type Output = HandlerService<E>;

    async fn transform(&self, service: HandlerService<E>) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(id, LoggingService { inner: service })
    }
}

/// Constructs the ping route and wraps it with response and logging middleware.
fn build_app() -> AppResult<App> {
    App::new()?
        .serializer(BincodeSerializer)
        .route(PING_ID, Arc::new(|_: &Envelope| Box::pin(ping_handler())))?
        .wrap(PongMiddleware)?
        .wrap(Logging)
}

/// Default listener address used when no command-line address is supplied.
const DEFAULT_ADDR: &str = "127.0.0.1:7878";

/// Parses an optional listener address, falling back to the documented default.
fn parse_server_addr() -> std::io::Result<SocketAddr> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());
    addr.parse().map_err(std::io::Error::other)
}

/// Starts the server and keeps it alive until the shared shutdown signal fires.
/// The server loop owns listener shutdown, ensuring accepted tasks finish in the
/// same lifecycle as the example process.
async fn run() -> std::io::Result<()> {
    runtime_bootstrap::init_tracing();
    let app = runtime_bootstrap::build_runtime_app(build_app).await?;
    let listener = runtime_bootstrap::bind_listener(parse_server_addr()?).await?;
    runtime_bootstrap::serve_until_shutdown(
        listener,
        app,
        "ping-pong server received shutdown signal",
    )
    .await
}

fn main() -> std::io::Result<()> { runtime_bootstrap::run_current_thread(run()) }
