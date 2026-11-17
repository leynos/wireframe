//! A response handler may read its request after an `await`.
//!
//! The returned future is tied to the borrowed request, so a handler can
//! inspect request data once an asynchronous operation has completed without
//! copying it first.
use std::sync::Arc;

use wireframe::{
    app::{Envelope, Packet, ResponseHandler, WireframeApp},
    response::Response,
    serializer::BincodeSerializer,
};

/// Echo the request after yielding once, so the future borrows the request.
async fn echo_after_yield(request: &Envelope) -> Response<Envelope, ()> {
    tokio::task::yield_now().await;
    Response::Single(Envelope::new(
        request.id(),
        None,
        request.payload_bytes().to_vec(),
    ))
}

fn main() {
    let handler: ResponseHandler<Envelope> =
        Arc::new(|request: &Envelope| Box::pin(echo_after_yield(request)));
    let _app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("the application should build")
        .route_response(7, handler)
        .expect("the route should register");
}
