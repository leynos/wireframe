//! Tests ensuring middleware registration order is reversed during execution.
//!
//! Verifies tags are applied in reverse to request and response bodies.
#![cfg(not(loom))]

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use wireframe::{
    app::{Envelope, Handler},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{decode_frames, encode_frame};

mod common;
use common::TestResult;

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
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn middleware_applied_in_reverse_order() -> TestResult<()> {
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
    let bytes = serializer.serialize(&env)?;
    let mut codec = app.length_codec();
    let frame = encode_frame(&mut codec, bytes);
    client.write_all(&frame).await?;
    client.shutdown().await?;

    let handle = tokio::spawn(async move { app.handle_connection_result(server).await });

    let mut out = Vec::new();
    client.read_to_end(&mut out).await?;
    handle.await??;

    let frames = decode_frames(out);
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp, _) = serializer.deserialize::<Envelope>(first)?;
    let parts = wireframe::app::Packet::into_parts(resp);
    let correlation_id = parts.correlation_id();
    let payload = parts.into_payload();
    assert_eq!(
        payload,
        [b'X', b'A', b'B', b'B', b'A'],
        "unexpected payload"
    );
    assert_eq!(correlation_id, Some(7), "unexpected correlation id");
    Ok(())
}
