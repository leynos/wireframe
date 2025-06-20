use std::io;

use wireframe::{
    app::{Middleware, WireframeApp},
    server::WireframeServer,
};

/// Simple middleware demonstrating the `wrap` API.
///
/// `Middleware` has no hooks yet, so this type is just a marker.
struct Logger;
impl Middleware for Logger {}

#[tokio::main]
async fn main() -> io::Result<()> {
    let factory = || {
        WireframeApp::new()
            .unwrap()
            .wrap(Logger)
            .unwrap()
            .route(
                1,
                Box::new(|_| {
                    Box::pin(async move {
                        println!("echo request received");
                        // `WireframeApp` automatically echoes the envelope back.
                    })
                }),
            )
            .unwrap()
    };

    WireframeServer::new(factory)
        .bind("127.0.0.1:7878".parse().unwrap())?
        .run()
        .await
}
