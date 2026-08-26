//! Runtime bootstrap shared by TCP server examples.

use std::{future::Future, net::SocketAddr, sync::Arc};

use tokio::net::{TcpListener, TcpStream};
use tracing::error;
use wireframe::{app::Envelope, serializer::BincodeSerializer};

use crate::server_loop;

/// Concrete application handle shared by the TCP examples' bootstrap helpers.
/// Keeping the alias here ensures each example wires the same envelope and
/// serializer contract into its runtime and connection tasks.
type ExampleApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;
type PreparedExampleApp = wireframe::app::PreparedApp<BincodeSerializer, (), Envelope>;
/// Initialize tracing for examples, ignoring duplicate global subscriber setup.
pub(crate) fn init_tracing() { let _ = tracing_subscriber::fmt::try_init(); }

/// Convert an example app builder into a shared runtime app handle.
pub(crate) async fn build_runtime_app(
    build_app: impl FnOnce() -> wireframe::app::Result<ExampleApp>,
) -> std::io::Result<Arc<PreparedExampleApp>> {
    let app = build_app().map_err(|error| std::io::Error::other(error.to_string()))?;
    let app = app.prepare().await.map_err(std::io::Error::other)?;
    Ok(Arc::new(app))
}

/// Bind a TCP listener for an already parsed socket address.
pub(crate) async fn bind_listener(addr: SocketAddr) -> std::io::Result<TcpListener> {
    TcpListener::bind(addr).await
}

/// Spawn one accepted TCP stream onto the shared app.
pub(crate) fn spawn_connection(app: Arc<PreparedExampleApp>, stream: TcpStream) {
    tokio::spawn(async move {
        if let Err(error) = app.handle_connection_result(stream).await {
            error!("connection handling failed: {error}");
        }
    });
}

/// Accept connections until shutdown and dispatch each stream to the app.
pub(crate) async fn serve_until_shutdown(
    listener: TcpListener,
    app: Arc<PreparedExampleApp>,
    shutdown_message: &'static str,
) -> std::io::Result<()> {
    while let Some(stream) = server_loop::accept_until_shutdown(&listener, shutdown_message).await?
    {
        spawn_connection(Arc::clone(&app), stream);
    }

    Ok(())
}

/// Run an async example on a current-thread Tokio runtime.
pub(crate) fn run_current_thread(
    future: impl Future<Output = std::io::Result<()>>,
) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(future)
}
