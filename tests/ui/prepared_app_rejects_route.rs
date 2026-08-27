//! Compile-fail coverage for the `PreparedApp` route-registration boundary.
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    serializer::BincodeSerializer,
};

#[tokio::main]
async fn main() {
    let handler: Handler<Envelope> = std::sync::Arc::new(|_| Box::pin(async {}));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("builder")
        .prepare()
        .await
        .expect("prepared");
    let _ = prepared.route(1, handler);
}
