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
type EchoHandler =
    Arc<dyn Fn(&Envelope) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

use std::{io, net::SocketAddr, pin::Pin, sync::Arc};

use tokio::signal;
use tracing::{error, info};

fn echo_handler() -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async {
        info!("echo request received");
        // `WireframeApp` automatically echoes the envelope back.
    })
}

fn build_app(handler: EchoHandler) -> wireframe::app::Result<App> { App::new()?.route(1, handler) }

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    tracing_subscriber::fmt::init();

    let handler: EchoHandler = Arc::new(|_: &Envelope| echo_handler());
    build_app(handler.clone()).map_err(|err| {
        error!("failed to build echo app: {err}");
        ServerError::Bind(io::Error::other(err))
    })?;

    let factory = {
        let handler = Arc::clone(&handler);
        move || match build_app(Arc::clone(&handler)) {
            Ok(app) => app,
            Err(err) => {
                error!("failed to rebuild echo app: {err}");
                App::default()
            }
        }
    };

    let addr: SocketAddr = "127.0.0.1:7878".parse().map_err(|err| {
        ServerError::Bind(std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
    })?;
    let server = WireframeServer::new(factory).bind(addr)?;

    server
        .run_with_shutdown(async {
            match signal::ctrl_c().await {
                Ok(()) => info!("shutdown signal received, stopping echo server"),
                Err(err) => error!("failed to wait for shutdown signal: {err}"),
            }
        })
        .await?;
    Ok(())
}
