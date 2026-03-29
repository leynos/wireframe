//! Example demonstrating enumerated packet types with middleware routing.
//!
//! The application defines an enum representing different packet variants and
//! shows how to dispatch handlers based on the variant received.

use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin, sync::Arc};

use async_trait::async_trait;
use tokio::{
    net::{TcpListener, TcpStream},
    signal,
};
use tracing::{error, info, warn};
use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
};

type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

const DEFAULT_ADDR: &str = "127.0.0.1:7879";

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
enum ExamplePacket {
    Ping,
    Chat { user: String, msg: String },
    Stats(Vec<u32>),
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Frame {
    headers: HashMap<String, String>,
    packet: ExamplePacket,
}

/// Middleware that decodes incoming frames and logs packet details.
struct DecodeMiddleware;

/// Service wrapper that handles frame decoding before invoking the inner service.
struct DecodeService<S> {
    inner: S,
}

#[async_trait]
impl<S> Service for DecodeService<S>
where
    S: Service<Error = std::convert::Infallible> + Send + Sync,
{
    type Error = S::Error;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        match Frame::from_bytes(req.frame()) {
            Ok((frame, _)) => match frame.packet {
                ExamplePacket::Ping => info!("ping: {:?}", frame.headers),
                ExamplePacket::Chat { user, msg } => info!("{user} says: {msg}"),
                ExamplePacket::Stats(values) => info!("stats: {values:?}"),
            },
            Err(e) => {
                warn!("Failed to decode frame: {e}");
            }
        }

        let response = self.inner.call(req).await?;
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for DecodeMiddleware {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(id, DecodeService { inner: service })
    }
}

fn handle_packet(_env: &Envelope) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        info!("packet received");
    })
}

fn build_app() -> wireframe::app::Result<App> {
    App::new()?
        .wrap(DecodeMiddleware)?
        .route(1, Arc::new(handle_packet))
}

fn init_tracing() { let _ = tracing_subscriber::fmt::try_init(); }

fn build_runtime_app() -> std::io::Result<Arc<App>> {
    build_app()
        .map(Arc::new)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

fn parse_server_addr() -> std::io::Result<SocketAddr> {
    let addr_str = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    addr_str.parse().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("SERVER_ADDR must be a valid socket address: {error}"),
        )
    })
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
    info!("packet_enum server received shutdown signal");
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
