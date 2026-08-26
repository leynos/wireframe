//! Integration coverage for one-time application preparation.

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use wireframe::{
    app::{Envelope, Handler, PreparedApp, WireframeApp},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestResult, decode_frames, drive_prepared_with_frames, encode_frame};

type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;
type TestPreparedApp = PreparedApp<BincodeSerializer, (), Envelope>;

struct TransformCountingMiddleware {
    tag: u8,
    transforms: Arc<AtomicUsize>,
}

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

fn handler() -> Handler<Envelope> { Arc::new(|_envelope: &Envelope| Box::pin(async {})) }

fn build_frame(id: u32, payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let envelope = Envelope::new(id, Some(7), payload);
    let payload = serializer.serialize(&envelope)?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

fn response_payload(bytes: Vec<u8>) -> TestResult<Vec<u8>> {
    let frames = decode_frames(bytes)?;
    let [frame] = frames.as_slice() else {
        return Err("expected one response frame".into());
    };
    let serializer = BincodeSerializer;
    let (response, _) = serializer.deserialize::<Envelope>(frame)?;
    Ok(wireframe::app::Packet::into_parts(response).into_payload())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions make transform counts and middleware order failures explicit"
)]
async fn prepared_app_transforms_routes_once_and_reuses_them() -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let app = TestApp::new()?
        .route(1, handler())?
        .route(2, handler())?
        .wrap(TransformCountingMiddleware {
            tag: b'A',
            transforms: Arc::clone(&transforms),
        })?
        .wrap(TransformCountingMiddleware {
            tag: b'B',
            transforms: Arc::clone(&transforms),
        })?;

    assert_eq!(transforms.load(Ordering::SeqCst), 0);
    let prepared: TestPreparedApp = app
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    assert_eq!(transforms.load(Ordering::SeqCst), 4);

    let first = drive_prepared_with_frames(&prepared, vec![build_frame(1, vec![b'X'])?]).await?;
    let second = drive_prepared_with_frames(&prepared, vec![build_frame(2, vec![b'Y'])?]).await?;

    assert_eq!(transforms.load(Ordering::SeqCst), 4);
    assert_eq!(response_payload(first)?, [b'X', b'A', b'B', b'B', b'A']);
    assert_eq!(response_payload(second)?, [b'Y', b'A', b'B', b'B', b'A']);
    Ok(())
}
