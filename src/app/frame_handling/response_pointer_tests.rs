//! Pointer identity across the production response dispatch seam.
//!
//! [`forward_response`](super::forward_response) hands the inbound envelope's
//! payload to `ServiceRequest::new` and then to `HandlerService::call`. Both
//! take the buffer by value, so an adapter that started taking a slice and
//! rebuilding a vector would copy every payload without changing a single
//! frame count. These assertions pin the two steps that touch the buffer.
//!
//! # Scope
//!
//! Driving the whole `forward_response` needs an actor, a frame pipeline, a
//! codec, and a protocol. The fixture that does so lives in
//! [`super::terminator_tests`] and is larger than this plan's 150-line
//! threshold for new fixture code, so this module asserts the identity across
//! the two production steps that own the payload buffer, as risk R-4 of
//! `docs/execplans/10-2-1-capture-baselines.md` permits. Item `13.2.2` owns the
//! end-to-end form, and the allocation-level half of the guarantee lives in
//! `wireframe_testing/tests/pipeline_allocation_invariants`.

use std::convert::Infallible;

use crate::{
    app::Envelope,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse},
};

/// Payload size the assertions use.
///
/// Large enough that a copy is unmistakable to the allocation invariants, which
/// state their byte bounds against the same class.
const PAYLOAD_BYTES: usize = 64 * 1024;

/// Byte the payload is filled with, so a rewritten buffer would be visible.
const FILL: u8 = 0xab;

/// Handler that returns the request frame unchanged.
#[derive(Clone, Copy)]
struct EchoService;

#[async_trait::async_trait]
impl Service for EchoService {
    type Error = Infallible;

    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let correlation_id = request.correlation_id();
        Ok(ServiceResponse::new(request.into_inner(), correlation_id))
    }
}

/// The production dispatch seam must move the payload, not copy it.
#[tokio::test]
async fn the_handler_service_hands_back_the_envelope_buffer() {
    let payload = vec![FILL; PAYLOAD_BYTES];
    let arrival = payload.as_ptr();
    let request = ServiceRequest::new(payload, Some(3));
    assert_eq!(
        request.frame().as_ptr(),
        arrival,
        "building a request from the envelope payload must not copy it"
    );

    let service = HandlerService::<Envelope>::from_service(7, EchoService);
    let response = service
        .call(request)
        .await
        .expect("the echo service is infallible");

    assert_eq!(
        response.correlation_id(),
        Some(3),
        "the request's correlation identifier must survive dispatch"
    );
    assert_eq!(
        response.frame().as_ptr(),
        arrival,
        "dispatching through the handler service must not copy the payload"
    );

    let returned = response.into_inner();
    assert_eq!(
        returned.len(),
        PAYLOAD_BYTES,
        "the handler must receive the whole payload"
    );
    assert!(
        returned.iter().all(|byte| *byte == FILL),
        "the handler must receive the payload unchanged"
    );
}
