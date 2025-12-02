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

use std::{net::SocketAddr, pin::Pin};

use tracing::info;

fn echo_handler() -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async {
        info!("echo request received");
        // `WireframeApp` automatically echoes the envelope back.
    })
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    tracing_subscriber::fmt::init();

    let handler = std::sync::Arc::new(
        |_: &Envelope| -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> { echo_handler() },
    );
    let build_app = {
        let handler = handler.clone();
        move || match App::default().route(1, handler.clone()) {
            Ok(app) => app,
            Err(err) => {
                eprintln!("failed to build echo app: {err}");
                std::process::exit(1);
            }
        }
    };
    let factory = { move || build_app() };

    let addr: SocketAddr = "127.0.0.1:7878".parse().map_err(|err| {
        ServerError::Bind(std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
    })?;
    WireframeServer::new(factory).bind(addr)?.run().await?;
    Ok(())
}
