//! Tests for middleware transformations.
//!
//! Confirm that a custom middleware can modify requests and responses.

use async_trait::async_trait;
use wireframe::middleware::{Next, Service, ServiceRequest, ServiceResponse, Transform};

struct EchoService;

#[async_trait]
impl Service for EchoService {
    type Error = std::convert::Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let cid = req.correlation_id();
        Ok(ServiceResponse::new(req.into_inner(), cid))
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

    async fn transform(&self, service: S) -> Self::Output { ModifyService { inner: service } }
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
    let service = EchoService;
    let mw = ModifyMiddleware;
    let wrapped = mw.transform(service).await;

    let request = ServiceRequest::new(vec![1, 2, 3], Some(0));
    let response = wrapped.call(request).await.expect("middleware call failed");
    assert_eq!(response.frame(), &[1, 2, 3, b'!', b'?']);
}

#[tokio::test]
async fn test_modify_middleware_preserves_nonzero_correlation_id() {
    let service = EchoService;
    let mw = ModifyMiddleware;
    let wrapped = mw.transform(service).await;

    let correlation_id = Some(42);
    let request = ServiceRequest::new(vec![4, 5, 6], correlation_id);
    let response = wrapped.call(request).await.expect("middleware call failed");
    assert_eq!(response.frame(), &[4, 5, 6, b'!', b'?']);
    assert_eq!(response.correlation_id(), correlation_id);
}

#[tokio::test]
async fn middleware_preserves_none_correlation_id() {
    let service = EchoService;
    let mw = ModifyMiddleware;
    let wrapped = mw.transform(service).await;

    let request = ServiceRequest::new(vec![7, 8, 9], None);
    let response = wrapped.call(request).await.expect("middleware call failed");
    assert_eq!(response.frame(), &[7, 8, 9, b'!', b'?']);
    assert_eq!(response.correlation_id(), None);
}
