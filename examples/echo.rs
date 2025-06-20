use std::io;

use wireframe::{app::WireframeApp, server::WireframeServer};

#[tokio::main]
async fn main() -> io::Result<()> {
    let factory = || {
        WireframeApp::new()
            .unwrap()
            .route(
                1,
                std::sync::Arc::new(|_| {
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
