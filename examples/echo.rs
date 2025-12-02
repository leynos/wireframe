//! Minimal echo server example using `WireframeApp`.
//!
//! The application listens for incoming frames and simply echoes each
//! envelope back to the client.

use wireframe::{
    app::Envelope,
    serializer::BincodeSerializer,
    server::{ServerError, WireframeServer},
};

type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

use std::pin::Pin;

fn echo_handler() -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async {
        println!("echo request received");
        // `WireframeApp` automatically echoes the envelope back.
    })
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    let handler = std::sync::Arc::new(
        |_: &Envelope| -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> { echo_handler() },
    );
    let factory = {
        let handler = handler.clone();
        move || {
            App::new()
                .expect("failed to create WireframeApp")
                .route(1, handler.clone())
                .expect("failed to register route 1")
        }
    };

    WireframeServer::new(factory)
        .bind("127.0.0.1:7878".parse().expect("invalid socket address"))?
        .run()
        .await?;
    Ok(())
}
