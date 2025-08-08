//! Tests ensuring middleware registration order is reversed during execution.
//!
//! Verifies tags are applied in reverse to request and response bodies.

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use wireframe::{
    app::{Envelope, Handler, Packet, WireframeApp},
    frame::{FrameProcessor, LengthPrefixedProcessor},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
};

struct TagMiddleware(u8);

struct TagService<S> {
    inner: S,
    tag: u8,
}

#[async_trait]
impl<S> Service for TagService<S>
where
    S: Service<Error = std::convert::Infallible> + Send + Sync + 'static,
{
    type Error = std::convert::Infallible;

    async fn call(&self, mut req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        req.frame_mut().push(self.tag);
        let mut response = self.inner.call(req).await?;
        response.frame_mut().push(self.tag);
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for TagMiddleware {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(
            id,
            TagService {
                inner: service,
                tag: self.0,
            },
        )
    }
}

#[tokio::test]
async fn middleware_applied_in_reverse_order() {
    let handler: Handler<Envelope> = std::sync::Arc::new(|_env: &Envelope| Box::pin(async {}));
    let app = WireframeApp::new()
        .expect("failed to create app")
        .route(1, handler)
        .expect("route registration failed")
        .wrap(TagMiddleware(b'A'))
        .expect("wrap failed")
        .wrap(TagMiddleware(b'B'))
        .expect("wrap failed");

    let (mut client, server) = duplex(256);

    let env = Envelope::new(1, Some(7), vec![b'X']);
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(&env).expect("serialization failed");
    // Use the default 4-byte big-endian length prefix for framing
    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::new();
    processor.encode(&bytes, &mut buf).expect("encoding failed");
    client.write_all(&buf).await.expect("write failed");
    client.shutdown().await.expect("shutdown failed");

    let handle = tokio::spawn(async move { app.handle_connection(server).await });

    let mut out = Vec::new();
    client.read_to_end(&mut out).await.expect("read failed");
    handle.await.expect("join failed");

    let mut buf = BytesMut::from(&out[..]);
    let frame = processor
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (resp, _) = serializer
        .deserialize::<Envelope>(&frame)
        .expect("deserialize failed");
    let parts = resp.into_parts();
    assert_eq!(parts.payload, vec![b'X', b'A', b'B', b'B', b'A']);
    assert_eq!(parts.correlation_id, Some(7));
}
