//! Ping/pong example exchanging typed request and response packets.
//!
//! Demonstrates custom packet structs and middleware that maps `Ping` to
//! `Pong` responses.

use std::{net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use tokio::{net::TcpListener, signal};
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

#[tokio::main]
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! macro expansion performs modulo internally"
)]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let default_addr = "127.0.0.1:7878";
    let addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| default_addr.into());

    let app = Arc::new(build_app().map_err(std::io::Error::other)?);
    let addr: SocketAddr = addr_str.parse().map_err(std::io::Error::other)?;
    let listener = TcpListener::bind(addr).await?;
    loop {
        tokio::select! {
            res = listener.accept() => {
                let (stream, _) = res?;
                let app = Arc::clone(&app);
                tokio::spawn(async move {
                        if let Err(err) = app.handle_connection_result(stream).await {
                            error!("connection handling failed: {err}");
                        }
                });
            }
            ctrl_c = signal::ctrl_c() => {
                match ctrl_c {
                    Ok(()) => info!("ping-pong server received shutdown signal"),
                    Err(e) => error!("failed waiting for shutdown signal: {e}"),
                }
                break;
            }
        }
    }
    Ok(())
}
