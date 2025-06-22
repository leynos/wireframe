use std::{io, sync::Arc};

use async_trait::async_trait;
use wireframe::{
    app::WireframeApp,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
    server::WireframeServer,
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Ping(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Pong(u32);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct ErrorMsg(String);

const PING_ID: u32 = 1;

/// Handler invoked for `PING_ID` messages.
///
/// The middleware chain generates the actual response, so this
/// handler intentionally performs no work.
#[allow(clippy::unused_async)]
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
        let (ping_req, _) = match Ping::from_bytes(req.frame()) {
            Ok(val) => val,
            Err(e) => {
                eprintln!("failed to decode ping: {e:?}");
                let err = ErrorMsg(format!("decode error: {e:?}"));
                let bytes = err.to_bytes().unwrap_or_default();
                return Ok(ServiceResponse::new(bytes));
            }
        };
        let mut response = self.inner.call(req).await?;
        let pong_resp = Pong(ping_req.0 + 1);
        match pong_resp.to_bytes() {
            Ok(bytes) => *response.frame_mut() = bytes,
            Err(e) => {
                eprintln!("failed to encode pong: {e:?}");
                let err = ErrorMsg(format!("encode error: {e:?}"));
                let bytes = err.to_bytes().unwrap_or_default();
                return Ok(ServiceResponse::new(bytes));
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
        HandlerService::from_service(id, LoggingService { inner: service })
    }
}

fn build_app() -> WireframeApp {
    WireframeApp::new()
        .expect("failed to create app")
        .serializer(BincodeSerializer)
        .route(PING_ID, Arc::new(|_| Box::pin(ping_handler())))
        .expect("failed to register ping handler")
        .wrap(PongMiddleware)
        .expect("failed to apply PongMiddleware")
        .wrap(Logging)
        .expect("failed to apply Logging middleware")
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let factory = || build_app();

    let addr = "127.0.0.1:7878"
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    WireframeServer::new(factory).bind(addr)?.run().await
}
