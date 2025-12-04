//! Example demonstrating enumerated packet types with middleware routing.
//!
//! The application defines an enum representing different packet variants and
//! shows how to dispatch handlers based on the variant received.

use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin, sync::Arc};

use async_trait::async_trait;
use tokio::{net::TcpListener, signal};
use tracing::{info, warn};
use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
};

type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

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

#[tokio::main]
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! macro expansion performs modulo internally"
)]
async fn main() -> std::io::Result<()> {
    let app = Arc::new(build_app().map_err(std::io::Error::other)?);

    let addr: SocketAddr = std::env::var("SERVER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7879".to_string())
        .parse()
        .map_err(std::io::Error::other)?;

    let listener = TcpListener::bind(addr).await?;
    loop {
        tokio::select! {
            res = listener.accept() => {
                let (stream, _) = res?;
                let app = Arc::clone(&app);
                tokio::spawn(async move {
                    app.handle_connection(stream).await;
                });
            }
            _ = signal::ctrl_c() => {
                info!("packet_enum server received shutdown signal");
                break;
            }
        }
    }

    Ok(())
}
