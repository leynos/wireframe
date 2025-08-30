//! Tests ensuring middleware registration order is reversed during execution.
//!
//! Verifies tags are applied in reverse to request and response bodies.

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Handler},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::TEST_MAX_FRAME;

type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

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
    let app = TestApp::new()
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
    // Use a length-delimited codec for framing
    let mut codec = LengthDelimitedCodec::builder()
        .max_frame_length(TEST_MAX_FRAME)
        .new_codec();
    let mut buf = BytesMut::new();
    codec
        .encode(bytes.into(), &mut buf)
        .expect("encoding failed");
    client.write_all(&buf).await.expect("write failed");
    client.shutdown().await.expect("shutdown failed");

    let handle = tokio::spawn(async move { app.handle_connection(server).await });

    let mut out = Vec::new();
    client.read_to_end(&mut out).await.expect("read failed");
    handle.await.expect("join failed");

    let mut buf = BytesMut::from(&out[..]);
    let frame = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    assert!(buf.is_empty(), "unexpected trailing bytes after frame");
    let (resp, _) = serializer
        .deserialize::<Envelope>(&frame)
        .expect("deserialize failed");
    let parts = wireframe::app::Packet::into_parts(resp);
    let correlation_id = parts.correlation_id();
    let payload = parts.payload();
    assert_eq!(payload, vec![b'X', b'A', b'B', b'B', b'A']);
    assert_eq!(correlation_id, Some(7));
}
