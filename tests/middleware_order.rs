use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
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
impl Transform<HandlerService> for TagMiddleware {
    type Output = HandlerService;

    async fn transform(&self, service: HandlerService) -> Self::Output {
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
    let handler: Handler = std::sync::Arc::new(|_env: &Envelope| Box::pin(async {}));
    let app = WireframeApp::new()
        .unwrap()
        .route(1, handler)
        .unwrap()
        .wrap(TagMiddleware(b'A'))
        .unwrap()
        .wrap(TagMiddleware(b'B'))
        .unwrap();

    let (mut client, server) = duplex(256);

    let env = Envelope::new(1, vec![b'X']);
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(&env).unwrap();
    let processor = LengthPrefixedProcessor;
    let mut buf = BytesMut::new();
    processor.encode(&bytes, &mut buf).unwrap();
    client.write_all(&buf).await.unwrap();
    client.shutdown().await.unwrap();

    let handle = tokio::spawn(async move { app.handle_connection(server).await });

    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    handle.await.unwrap();

    let mut buf = BytesMut::from(&out[..]);
    let frame = processor.decode(&mut buf).unwrap().unwrap();
    let (resp, _) = serializer.deserialize::<Envelope>(&frame).unwrap();
    let (_, bytes) = resp.into_parts();
    assert_eq!(bytes, vec![b'X', b'A', b'B', b'B', b'A']);
}
