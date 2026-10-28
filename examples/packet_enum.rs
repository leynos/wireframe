//! Example demonstrating enumerated packet types with middleware routing.
//!
//! The application defines an enum representing different packet variants and
//! shows how to dispatch handlers based on the variant received.

use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin, sync::Arc};

use async_trait::async_trait;
use mockable::{DefaultEnv, Env};
use tokio::signal;
use tracing::{info, warn};
use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
    server::WireframeServer,
};

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

/// Validate a listener address, falling back when the environment lookup fails.
fn parse_server_addr(addr: &Result<String, std::env::VarError>) -> std::io::Result<SocketAddr> {
    let addr_str = addr.as_deref().unwrap_or(DEFAULT_ADDR);
    addr_str.parse().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("SERVER_ADDR must be a valid socket address: {error}"),
        )
    })
}

/// Initialize tracing, bind the listener, and serve until shutdown is signalled.
async fn run() -> std::io::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let app = build_app().map_err(|error| std::io::Error::other(error.to_string()))?;
    let server = WireframeServer::from_app(app)
        .bind(parse_server_addr(&DefaultEnv.raw("SERVER_ADDR"))?)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    server
        .run_with_shutdown(async {
            match signal::ctrl_c().await {
                Ok(()) => info!("packet_enum server received shutdown signal"),
                Err(error) => warn!("failed waiting for shutdown signal: {error}"),
            }
        })
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))
}

fn main() -> std::io::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

#[cfg(test)]
mod tests {
    //! Verifies listener address parsing without mutating the process environment.

    use std::{env::VarError, ffi::OsString, io, net::SocketAddr};

    use rstest::rstest;

    use super::{DEFAULT_ADDR, parse_server_addr};

    #[rstest]
    #[case::missing(VarError::NotPresent)]
    #[case::non_unicode(VarError::NotUnicode(OsString::from("raw value")))]
    fn uses_default_when_lookup_fails(#[case] error: VarError) -> Result<(), io::Error> {
        let addr = parse_server_addr(&Err(error))?;
        assert_eq!(addr.to_string(), DEFAULT_ADDR);
        Ok(())
    }

    #[test]
    fn accepts_configured_address() -> Result<(), io::Error> {
        let addr = parse_server_addr(&Ok("127.0.0.1:9000".to_string()))?;
        assert_eq!(addr, SocketAddr::from(([127, 0, 0, 1], 9000)));
        Ok(())
    }

    #[test]
    fn rejects_malformed_configured_address() -> Result<(), io::Error> {
        let error = parse_server_addr(&Ok("not-an-address".to_string()))
            .expect_err("malformed configured address must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "SERVER_ADDR must be a valid socket address: invalid socket address syntax"
        );
        Ok(())
    }
}
