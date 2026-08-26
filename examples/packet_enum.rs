//! Example demonstrating enumerated packet types with middleware routing.
//!
//! The application defines an enum representing different packet variants and
//! shows how to dispatch handlers based on the variant received.

use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin, sync::Arc};

use async_trait::async_trait;
use tracing::{info, warn};
use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
};

#[path = "support/runtime_bootstrap.rs"]
mod runtime_bootstrap;
#[path = "support/server_loop.rs"]
mod server_loop;

/// Application type used by this example's middleware and route wiring.
type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

/// Fallback listener address used when `SERVER_ADDR` is not configured.
const DEFAULT_ADDR: &str = "127.0.0.1:7879";

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
/// Packet variants demonstrate dispatch after deserialization at the edge.
enum ExamplePacket {
    /// Health-style message with no payload.
    Ping,
    /// User-authored chat text, retained as owned strings for async handling.
    Chat {
        /// Display name used when logging the message.
        user: String,
        /// Message body carried through the decoded frame.
        msg: String,
    },
    /// A collection of counters decoded as one logical statistics message.
    Stats(Vec<u32>),
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
/// Wire payload combining transport metadata with one typed packet.
struct Frame {
    /// Metadata available to middleware for logging or routing decisions.
    headers: HashMap<String, String>,
    /// Deserialized application message selected by the packet discriminator.
    packet: ExamplePacket,
}

/// Middleware that decodes incoming frames and logs packet details.
struct DecodeMiddleware;

/// Service wrapper that handles frame decoding before invoking the inner service.
struct DecodeService<S> {
    /// Inner service invoked after decoding and observing the incoming frame.
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

/// Route callback used to acknowledge a packet after middleware inspection.
fn handle_packet(_env: &Envelope) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        info!("packet received");
    })
}

/// Assemble the example application with decoding middleware before its route.
fn build_app() -> wireframe::app::Result<App> {
    App::new()?
        .wrap(DecodeMiddleware)?
        .route(1, Arc::new(handle_packet))
}

/// Read and validate the listener address, reporting malformed configuration.
fn parse_server_addr() -> std::io::Result<SocketAddr> {
    let addr_str = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    addr_str.parse().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("SERVER_ADDR must be a valid socket address: {error}"),
        )
    })
}

/// Initialize tracing, bind the listener, and serve until shutdown is signalled.
async fn run() -> std::io::Result<()> {
    runtime_bootstrap::init_tracing();
    let app = runtime_bootstrap::build_runtime_app(build_app).await?;
    let listener = runtime_bootstrap::bind_listener(parse_server_addr()?).await?;
    runtime_bootstrap::serve_until_shutdown(
        listener,
        app,
        "packet_enum server received shutdown signal",
    )
    .await
}

fn main() -> std::io::Result<()> { runtime_bootstrap::run_current_thread(run()) }
