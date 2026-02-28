//! Client runtime for wireframe connections.
//!
//! This module provides a configurable client runtime that mirrors the
//! server's framing and serialization layers. The client supports connection
//! lifecycle hooks that mirror the server's hooks, enabling consistent
//! instrumentation across both ends of a wireframe connection.

use std::{io, sync::Arc};

use futures::future::BoxFuture;
use tokio::net::TcpStream;

mod builder;
mod codec_config;
mod config;
mod error;
mod hooks;
mod messaging;
mod preamble_builder;
mod preamble_exchange;
mod response_stream;
mod runtime;
mod send_streaming;
mod socket_option_methods;
mod streaming;
mod tracing_config;
mod tracing_helpers;

pub use builder::WireframeClientBuilder;
pub use codec_config::ClientCodecConfig;
pub use config::SocketOptions;
pub use error::{ClientError, ClientProtocolError, ClientWireframeError};
pub use hooks::{
    AfterReceiveHook,
    BeforeSendHook,
    ClientConnectionSetupHandler,
    ClientConnectionTeardownHandler,
    ClientErrorHandler,
};
pub use response_stream::ResponseStream;
pub use runtime::WireframeClient;
pub use send_streaming::{SendStreamingConfig, SendStreamingOutcome};
pub use tracing_config::TracingConfig;

/// Handler invoked after the client successfully writes its preamble.
///
/// The handler receives the sent preamble and a mutable reference to the TCP
/// stream. It may read the server's response preamble from the stream. Any
/// leftover bytes (read beyond the server's response) must be returned so they
/// can be replayed before framed communication begins.
///
/// # Example
///
/// ```ignore
/// use futures::FutureExt;
/// use wireframe::preamble::read_preamble;
///
/// let handler = |_preamble: &MyPreamble, stream: &mut TcpStream| {
///     async move {
///         let (response, leftover) = read_preamble::<_, ServerAck>(stream).await?;
///         // Validate response...
///         Ok(leftover)
///     }
///     .boxed()
/// };
/// ```
pub type ClientPreambleSuccessHandler<T> = Arc<
    dyn for<'a> Fn(&'a T, &'a mut TcpStream) -> BoxFuture<'a, io::Result<Vec<u8>>>
        + Send
        + Sync
        + 'static,
>;

/// Handler invoked when the client preamble exchange fails.
///
/// The handler receives the error and a mutable reference to the TCP stream,
/// allowing it to log or send an error response before the connection closes.
pub type ClientPreambleFailureHandler = Arc<
    dyn for<'a> Fn(&'a ClientError, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync
        + 'static,
>;

#[cfg(test)]
mod tests;
