//! End-to-end TCP coverage for immutable prepared applications.

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestResult, decode_frames, encode_frame};

type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Middleware that exposes transform and request-response execution counts.
struct TransformCountingMiddleware {
    transforms: Arc<AtomicUsize>,
}

/// Service that tags requests and responses around its delegate.
struct TagService<S> {
    inner: S,
}

#[async_trait]
impl<S> Service for TagService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    /// Tag both sides of the delegated request-response exchange.
    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        request.frame_mut().push(b'A');
        let mut response = self.inner.call(request).await?;
        response.frame_mut().push(b'A');
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for TransformCountingMiddleware {
    type Output = HandlerService<Envelope>;

    /// Count transformation and wrap the route service once.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.transforms.fetch_add(1, Ordering::SeqCst);
        HandlerService::from_service(service.id(), TagService { inner: service })
    }
}

/// Build a handler that accepts an envelope without changing it.
fn handler() -> Handler<Envelope> { Arc::new(|_envelope| Box::pin(async {})) }

/// Encode an envelope into the default transport frame.
fn build_frame(payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let envelope = Envelope::new(1, Some(7), payload);
    let payload = serializer.serialize(&envelope)?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

/// Decode the response envelope and return its payload.
fn response_payload(bytes: Vec<u8>) -> TestResult<Vec<u8>> {
    let frames = decode_frames(bytes)?;
    let [frame] = frames.as_slice() else {
        return Err("expected one response frame".into());
    };
    let serializer = BincodeSerializer;
    let (response, _) = serializer.deserialize::<Envelope>(frame)?;
    Ok(wireframe::app::Packet::into_parts(response).into_payload())
}

/// Prepared applications serve TCP connections without rebuilding middleware.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions make prepared TCP dispatch and transform reuse explicit"
)]
async fn prepared_app_serves_tcp_connection_without_retransforming() -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let prepared = TestApp::new()?
        .route(1, handler())?
        .wrap(TransformCountingMiddleware {
            transforms: Arc::clone(&transforms),
        })?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    assert_eq!(transforms.load(Ordering::SeqCst), 1);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        prepared.handle_connection_result(stream).await
    });

    let mut client = TcpStream::connect(address).await?;
    client.write_all(&build_frame(vec![b'X'])?).await?;
    client.shutdown().await?;
    let mut response = Vec::new();
    client.read_to_end(&mut response).await?;
    server.await??;

    assert_eq!(response_payload(response)?, [b'X', b'A', b'A']);
    assert_eq!(transforms.load(Ordering::SeqCst), 1);
    Ok(())
}
