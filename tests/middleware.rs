//! Tests for middleware transformations.
//!
//! Confirm that a custom middleware can modify requests and responses.

use async_trait::async_trait;
use wireframe::middleware::{Next, Service, ServiceRequest, ServiceResponse, Transform};

struct EchoService(u32);

#[async_trait]
impl Service for EchoService {
    type Error = std::convert::Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        assert_eq!(req.correlation_id(), self.0);
        Ok(ServiceResponse::new(req.into_inner()))
    }
}

struct ModifyMiddleware;

struct ModifyService<S> {
    inner: S,
}

#[async_trait]
impl<S> Transform<S> for ModifyMiddleware
where
    S: Service + Send + Sync + 'static,
{
    type Output = ModifyService<S>;

    async fn transform(&self, service: S) -> Self::Output {
        ModifyService { inner: service }
    }
}

#[async_trait]
impl<S> Service for ModifyService<S>
where
    S: Service + Send + Sync + 'static,
{
    type Error = S::Error;

    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        request.frame_mut().push(b'!');
        let next = Next::new(&self.inner);
        let mut response = next.call(request).await?;
        response.frame_mut().push(b'?');
        Ok(response)
    }
}

#[tokio::test]
async fn middleware_modifies_request_and_response() {
    let corr_id = 7;
    let service = EchoService(corr_id);
    let mw = ModifyMiddleware;
    let wrapped = mw.transform(service).await;

    let request = ServiceRequest::new(vec![1, 2, 3], corr_id);
    let response = wrapped.call(request).await.expect("middleware call failed");
    assert_eq!(response.frame(), &[1, 2, 3, b'!', b'?']);
}
