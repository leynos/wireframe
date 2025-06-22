use std::{collections::HashMap, future::Future, io, pin::Pin};

use async_trait::async_trait;
use wireframe::{
    app::{Envelope, WireframeApp},
    frame::{LengthFormat, LengthPrefixedProcessor},
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    server::WireframeServer,
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
enum Packet {
    Ping,
    Chat { user: String, msg: String },
    Stats(Vec<u32>),
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Frame {
    headers: HashMap<String, String>,
    packet: Packet,
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
        if let Ok((frame, _)) = Frame::from_bytes(req.frame()) {
            match frame.packet {
                Packet::Ping => println!("ping: {:?}", frame.headers),
                Packet::Chat { user, msg } => println!("{user} says: {msg}"),
                Packet::Stats(values) => println!("stats: {values:?}"),
            }
        }
        let response = self.inner.call(req).await?;
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService> for DecodeMiddleware {
    type Output = HandlerService;

    async fn transform(&self, service: HandlerService) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(id, DecodeService { inner: service })
    }
}

fn handle_packet(_env: &Envelope) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        println!("packet received");
    })
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let factory = || {
        WireframeApp::new()
            .unwrap()
            .frame_processor(LengthPrefixedProcessor::new(LengthFormat::u16_le()))
            .wrap(DecodeMiddleware)
            .unwrap()
            .route(1, std::sync::Arc::new(handle_packet))
            .unwrap()
    };

    WireframeServer::new(factory)
        .bind("127.0.0.1:7879".parse().unwrap())?
        .run()
        .await
}
