//! Reusable fixtures for prepared-application integration tests.

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestResult, decode_frames, encode_frame};

/// Concrete application type shared by prepared-application integration tests.
pub type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Middleware that counts one-time transforms and tags request-response data.
pub struct TransformCountingMiddleware {
    /// Byte appended by the middleware around each handled frame.
    pub tag: u8,
    /// Number of one-time middleware transformations performed during preparation.
    pub transforms: Arc<AtomicUsize>,
}

/// Service that tags requests and responses around its delegate.
struct TagService<S> {
    inner: S,
    tag: u8,
}

#[async_trait]
impl<S> Service for TagService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    /// Add this middleware's tag before and after the delegated service.
    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        request.frame_mut().push(self.tag);
        let mut response = self.inner.call(request).await?;
        response.frame_mut().push(self.tag);
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for TransformCountingMiddleware {
    type Output = HandlerService<Envelope>;

    /// Count this one-time transform and wrap its service with a tag.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.transforms.fetch_add(1, Ordering::SeqCst);
        let id = service.id();
        HandlerService::from_service(
            id,
            TagService {
                inner: service,
                tag: self.tag,
            },
        )
    }
}

/// Build a handler that accepts an envelope without changing it.
pub fn handler() -> Handler<Envelope> { Arc::new(|_: &Envelope| Box::pin(async {})) }

/// Encode an envelope into a frame for the default test transport.
pub fn build_frame(id: u32, payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let envelope = Envelope::new(id, Some(7), payload);
    let payload = serializer.serialize(&envelope)?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

/// Decode a single response frame and return its envelope payload.
pub fn response_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let frames = decode_frames(bytes)?;
    let [frame] = frames.as_slice() else {
        return Err("expected one response frame".into());
    };
    let serializer = BincodeSerializer;
    let (response, _) = serializer.deserialize::<Envelope>(frame)?;
    Ok(wireframe::app::Packet::into_parts(response).into_payload())
}
