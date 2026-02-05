#![cfg_attr(loom, allow(missing_docs))]
#![cfg(not(loom))]
//! Tests for middleware transformations.
//!
//! Confirm that a custom middleware can modify requests and responses.

use async_trait::async_trait;
use rstest::rstest;
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

#[rstest(correlation_id => [None, Some(0), Some(42)])]
#[tokio::test]
async fn middleware_modifies_request_and_response_preserves_cid(correlation_id: Option<u64>) {
    let service = EchoService;
    let mw = ModifyMiddleware;
    let wrapped = mw.transform(service).await;

    let request = ServiceRequest::new(vec![1, 2, 3], correlation_id);
    let response = wrapped.call(request).await.expect("middleware call failed");

    assert_eq!(response.frame(), &[1, 2, 3, b'!', b'?']);
    assert_eq!(response.correlation_id(), correlation_id);
}

#[test]
fn service_request_setter_updates_correlation_id() {
    let mut req = ServiceRequest::new(vec![], None);
    let _ = req.set_correlation_id(Some(7));
    assert_eq!(req.correlation_id(), Some(7));

    let _ = req.set_correlation_id(None);
    assert_eq!(req.correlation_id(), None);
}
