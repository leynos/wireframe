use std::{io, sync::Arc};

use async_trait::async_trait;
use wireframe::{
    app::{Envelope, WireframeApp},
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
    server::WireframeServer,
};

/// Convenience helper to convert a service into a `HandlerService`.
fn wrap_service<S>(id: u32, svc: S) -> HandlerService
where
    S: Service<Error = std::convert::Infallible> + Send + Sync + 'static,
{
    HandlerService::from_service(id, svc)
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Ping(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Pong(u32);

const PING_ID: u32 = 1;

/// Handler invoked for `PING_ID` messages.
///
/// The middleware chain generates the actual response, so this
/// handler intentionally performs no work.
fn ping_handler(
    _env: &Envelope,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async {})
}

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
        let (ping_req, _) = match Ping::from_bytes(req.frame()) {
            Ok(val) => val,
            Err(e) => {
                eprintln!("failed to decode ping: {e:?}");
                return Ok(ServiceResponse::default());
            }
        };
        let mut response = self.inner.call(req).await?;
        let pong_resp = Pong(ping_req.0 + 1);
        match pong_resp.to_bytes() {
            Ok(bytes) => *response.frame_mut() = bytes,
            Err(e) => {
                eprintln!("failed to encode pong: {e:?}");
                return Ok(ServiceResponse::default());
            }
        }
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService> for PongMiddleware {
    type Output = HandlerService;

    async fn transform(&self, service: HandlerService) -> Self::Output {
        let id = service.id();
        wrap_service(id, PongService { inner: service })
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
        println!("request: {:?}", req.frame());
        let resp = self.inner.call(req).await?;
        println!("response: {:?}", resp.frame());
        Ok(resp)
    }
}

#[async_trait]
impl Transform<HandlerService> for Logging {
    type Output = HandlerService;

    async fn transform(&self, service: HandlerService) -> Self::Output {
        let id = service.id();
        wrap_service(id, LoggingService { inner: service })
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let factory = || {
        WireframeApp::new()
            .unwrap()
            .serializer(BincodeSerializer)
            .route(PING_ID, Arc::new(ping_handler))
            .unwrap()
            .wrap(PongMiddleware)
            .unwrap()
            .wrap(Logging)
            .unwrap()
    };

    WireframeServer::new(factory)
        .bind("127.0.0.1:7878".parse().unwrap())?
        .run()
        .await
}
