//! Ping/pong example exchanging typed request and response packets.
//!
//! Demonstrates custom packet structs and middleware that maps `Ping` to
//! `Pong` responses.

use std::{net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use tokio::{
    net::{TcpListener, TcpStream},
    signal,
};
use tracing::{error, info};
use wireframe::{
    app::{Envelope, Packet, Result as AppResult},
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
};

type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Ping(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Pong(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct ErrorMsg(String);

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

struct PongMiddleware;

struct PongService<S> {
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

struct Logging;

struct LoggingService<S> {
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

fn build_app() -> AppResult<App> {
    App::new()?
        .serializer(BincodeSerializer)
        .route(PING_ID, Arc::new(|_: &Envelope| Box::pin(ping_handler())))?
        .wrap(PongMiddleware)?
        .wrap(Logging)
}

const DEFAULT_ADDR: &str = "127.0.0.1:7878";

fn init_tracing() { let _ = tracing_subscriber::fmt::try_init(); }

fn parse_server_addr() -> std::io::Result<SocketAddr> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());
    addr.parse().map_err(std::io::Error::other)
}

fn build_runtime_app() -> std::io::Result<Arc<App>> {
    build_app()
        .map(Arc::new)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

async fn bind_listener() -> std::io::Result<TcpListener> {
    let addr = parse_server_addr()?;
    TcpListener::bind(addr).await
}

enum ServerEvent {
    Accepted(TcpStream),
    Shutdown,
}

fn accepted_event(
    result: std::io::Result<(TcpStream, SocketAddr)>,
) -> std::io::Result<ServerEvent> {
    let (stream, _) = result?;
    Ok(ServerEvent::Accepted(stream))
}

fn log_shutdown_success() {
    info!("ping-pong server received shutdown signal");
}

fn log_shutdown_error(error: &std::io::Error) {
    error!("failed waiting for shutdown signal: {error}");
}

fn shutdown_event(result: &std::io::Result<()>) -> ServerEvent {
    if let Some(error) = result.as_ref().err() {
        log_shutdown_error(error);
    } else {
        log_shutdown_success();
    }

    ServerEvent::Shutdown
}

#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! macro expansion performs modulo internally"
)]
async fn next_server_event(listener: &TcpListener) -> std::io::Result<ServerEvent> {
    tokio::select! {
        accept_result = listener.accept() => accepted_event(accept_result),
        shutdown_result = signal::ctrl_c() => Ok(shutdown_event(&shutdown_result)),
    }
}

fn spawn_connection(app: Arc<App>, stream: TcpStream) {
    tokio::spawn(async move {
        if let Err(error) = app.handle_connection_result(stream).await {
            error!("connection handling failed: {error}");
        }
    });
}

async fn accepted_stream(listener: &TcpListener) -> std::io::Result<Option<TcpStream>> {
    match next_server_event(listener).await? {
        ServerEvent::Accepted(stream) => Ok(Some(stream)),
        ServerEvent::Shutdown => Ok(None),
    }
}

async fn serve_until_shutdown(listener: TcpListener, app: Arc<App>) -> std::io::Result<()> {
    while let Some(stream) = accepted_stream(&listener).await? {
        spawn_connection(Arc::clone(&app), stream);
    }

    Ok(())
}

async fn run() -> std::io::Result<()> {
    init_tracing();
    let app = build_runtime_app()?;
    let listener = bind_listener().await?;
    serve_until_shutdown(listener, app).await
}

fn main() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}
