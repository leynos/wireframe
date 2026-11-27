//! Middleware double shared by the prepared-application integration tests.
//!
//! The double tags the frames that pass through the chain and counts the
//! one-time transforms performed while the application was prepared, so a test
//! can tell a route that ran the chain from one that never entered it. It lives
//! in its own module because a test target that only needs the double should
//! not have to compile the prepared-application helpers beside it.

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use wireframe::{
    app::Envelope,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
};

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
