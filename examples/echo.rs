//! Minimal echo server example using `WireframeApp`.
//!
//! The application listens for incoming frames and simply echoes each
//! envelope back to the client.

use wireframe::{app::Envelope, serializer::BincodeSerializer, server::WireframeServer};

/// Concrete application type used by the example's bincode-encoded envelope
/// route.
///
/// The unit state keeps the example stateless while `Envelope` preserves the
/// framework's request/response protocol at the application boundary.
type App = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

/// Thread-safe handler shape accepted by `WireframeApp::route`.
///
/// The shared callback can be cloned into each server-created application,
/// while the boxed future lets a route return asynchronous work without
/// imposing one concrete future type on the application.
type EchoHandler =
    Arc<dyn Fn(&Envelope) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

use std::{error::Error, net::SocketAddr, pin::Pin, sync::Arc};

use tokio::signal;
use tracing::{error, info};

/// Creates the asynchronous callback for the echo route.
///
/// The callback records receipt for observability; `WireframeApp` owns the
/// protocol response and echoes the incoming envelope after the handler has
/// completed. Keeping the handler side-effect-free makes the wire-level echo
/// behaviour explicit in this minimal example.
fn echo_handler() -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async {
        info!("echo request received");
        // `WireframeApp` automatically echoes the envelope back.
    })
}

/// Constructs an application and binds route `1` to the echo callback.
///
/// Application construction is fallible because serializer and route setup
/// can reject an invalid configuration; callers can therefore rebuild the
/// application without hiding that failure behind a panic.
fn build_app(handler: EchoHandler) -> wireframe::app::Result<App> { App::new()?.route(1, handler) }

/// Starts the echo server and waits for a Ctrl-C shutdown signal.
///
/// The completed application is prepared once by the server and shared by all
/// connections. Binding, application construction, and server execution errors
/// propagate to the command-line boundary for reporting.
async fn run() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let handler: EchoHandler = Arc::new(|_: &Envelope| echo_handler());
    let app = build_app(handler)
        .inspect_err(|err| error!("failed to build echo app: {err}"))
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    let addr: SocketAddr = "127.0.0.1:7878".parse()?;
    let server = WireframeServer::from_app(app).bind(addr)?;

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

fn main() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}
