//! Minimal echo server example using `WireframeApp`.
//!
//! The application listens for incoming frames and simply echoes each
//! envelope back to the client.

use wireframe::{
    app::{Envelope, WireframeApp},
    server::{ServerError, WireframeServer},
};

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    let factory = || {
        WireframeApp::new()
            .expect("failed to create WireframeApp")
            .route(
                1,
                std::sync::Arc::new(|_: &Envelope| {
                    Box::pin(async move {
                        println!("echo request received");
                        // `WireframeApp` automatically echoes the envelope back.
                    })
                }),
            )
            .expect("failed to register route")
    };

    WireframeServer::new(factory)
        .bind("127.0.0.1:7878".parse().expect("invalid bind address"))?
        .run()
        .await?;
    Ok(())
}
