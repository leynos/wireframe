//! Compile-fail coverage for the `PreparedApp` route-registration boundary.
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    response::Response,
    serializer::BincodeSerializer,
};

#[tokio::main]
async fn main() {
    let handler: ResponseHandler<Envelope> = std::sync::Arc::new(|_| {
        Box::pin(async { Response::<Envelope, ()>::Empty })
    });
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("builder")
        .prepare()
        .await
        .expect("prepared");
    let _ = prepared.route_response(1, handler);
}