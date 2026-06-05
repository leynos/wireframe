//! Test application builders for fragment integration tests.

use std::io;

use tokio::sync::mpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Handler, Packet, WireframeApp},
    fragment::FragmentationConfig,
};

use super::{ROUTE_ID, TestResult};

const APP_DUPLEX_BUFFER_SIZE: usize = 8_192;

/// Create a handler that forwards received payloads to an unbounded channel.
///
/// # Panics
///
/// The returned handler panics if sending to the channel fails.
#[must_use]
pub fn make_handler(sender: &mpsc::UnboundedSender<Vec<u8>>) -> Handler<Envelope> {
    let tx = sender.clone();
    std::sync::Arc::new(move |env: &Envelope| {
        let tx = tx.clone();
        let payload = env.clone().into_parts().into_payload();
        Box::pin(async move {
            assert!(
                tx.send(payload).is_ok(),
                "handler channel send must succeed in tests"
            );
        })
    })
}

/// Create a test [`WireframeApp`] with fragmentation enabled.
///
/// # Errors
///
/// Returns an error if app creation or route registration fails.
pub fn make_app(
    capacity: usize,
    config: FragmentationConfig,
    sender: &mpsc::UnboundedSender<Vec<u8>>,
) -> TestResult<WireframeApp> {
    Ok(WireframeApp::new()?
        .buffer_capacity(capacity)
        .fragmentation(Some(config))
        .route(ROUTE_ID, make_handler(sender))?)
}

/// Spawn an app and return the client connection and server task handle.
pub fn spawn_app(
    app: WireframeApp,
) -> (
    Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(APP_DUPLEX_BUFFER_SIZE);
    let client = Framed::new(client_stream, codec.clone());
    let server = tokio::spawn(async move { app.handle_connection_result(server_stream).await });
    (client, server)
}
