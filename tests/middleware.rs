//! Tests for middleware transformations.
//!
//! Confirm that a custom middleware can modify requests and responses.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use rstest::rstest;
use wireframe::{
    app::Envelope,
    middleware::{
        HandlerService,
        Next,
        Service,
        ServiceRequest,
        ServiceResponse,
        Transform,
        from_fn,
    },
};

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

#[tokio::test]
async fn from_fn_clones_captured_state_across_routes() {
    let marker = Arc::new(*b"!?");
    let middleware = from_fn(
        move |mut request: ServiceRequest, _next: Next<'_, HandlerService<Envelope>>| {
            let [request_marker, response_marker] = marker.as_ref();
            request.frame_mut().push(*request_marker);
            let correlation_id = request.correlation_id();
            let mut frame = request.into_inner();
            frame.push(*response_marker);
            std::future::ready(Ok(ServiceResponse::new(frame, correlation_id)))
        },
    );

    let first_route = middleware
        .transform(HandlerService::from_service(1, EchoService))
        .await;
    let second_route = middleware
        .transform(HandlerService::from_service(2, EchoService))
        .await;

    let first_response = first_route
        .call(ServiceRequest::new(vec![b'X'], Some(7)))
        .await
        .expect("first route middleware call should succeed");
    let second_response = second_route
        .call(ServiceRequest::new(vec![b'Y'], Some(8)))
        .await
        .expect("second route middleware call should succeed");

    assert_eq!(first_response.frame(), b"X!?");
    assert_eq!(first_response.correlation_id(), Some(7));
    assert_eq!(second_response.frame(), b"Y!?");
    assert_eq!(second_response.correlation_id(), Some(8));
}

#[test]
fn service_request_setter_updates_correlation_id() {
    let mut req = ServiceRequest::new(vec![], None);
    let _ = req.set_correlation_id(Some(7));
    assert_eq!(req.correlation_id(), Some(7));

    let _ = req.set_correlation_id(None);
    assert_eq!(req.correlation_id(), None);
}

/// The `frame` accessor has no production callers, so a mutant returning a
/// leaked constant vector survives unless asserted directly.
#[test]
fn service_request_frame_borrows_original_bytes() {
    let req = ServiceRequest::new(vec![1, 2, 3], None);
    assert_eq!(req.frame(), &[1, 2, 3]);
}
