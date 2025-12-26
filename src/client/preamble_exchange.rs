//! Preamble exchange logic for wireframe client connections.
//!
//! This module handles writing the client preamble to the server and invoking
//! success/failure callbacks during the connection handshake phase.

use std::{io, sync::Arc, time::Duration};

use bincode::Encode;
use futures::future::BoxFuture;
use log::error;
use tokio::{net::TcpStream, time::timeout};

use super::{ClientError, ClientPreambleFailureHandler, ClientPreambleSuccessHandler};
use crate::preamble::write_preamble;

/// Holds optional preamble configuration for the client builder.
pub(crate) struct PreambleConfig<P> {
    pub(crate) preamble: P,
    pub(crate) on_success: Option<ClientPreambleSuccessHandler<P>>,
    pub(crate) on_failure: Option<ClientPreambleFailureHandler>,
    pub(crate) timeout: Option<Duration>,
}

impl<P> PreambleConfig<P>
where
    P: Encode + Send + Sync + 'static,
{
    /// Create a new preamble configuration with just the preamble value.
    pub(crate) fn new(preamble: P) -> Self {
        Self {
            preamble,
            on_success: None,
            on_failure: None,
            timeout: None,
        }
    }

    /// Set the timeout for the preamble exchange.
    pub(crate) fn set_timeout(&mut self, timeout: Duration) { self.timeout = Some(timeout); }

    /// Set the success handler.
    pub(crate) fn set_success_handler<H>(&mut self, handler: H)
    where
        H: for<'a> Fn(&'a P, &'a mut TcpStream) -> BoxFuture<'a, io::Result<Vec<u8>>>
            + Send
            + Sync
            + 'static,
    {
        self.on_success = Some(Arc::new(handler));
    }

    /// Set the failure handler.
    pub(crate) fn set_failure_handler<H>(&mut self, handler: H)
    where
        H: for<'a> Fn(&'a ClientError, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
            + Send
            + Sync
            + 'static,
    {
        self.on_failure = Some(Arc::new(handler));
    }
}

/// Perform the preamble exchange: write preamble, invoke success callback.
pub(crate) async fn perform_preamble_exchange<P>(
    stream: &mut TcpStream,
    config: PreambleConfig<P>,
) -> Result<Vec<u8>, ClientError>
where
    P: Encode + Send + Sync + 'static,
{
    let PreambleConfig {
        preamble,
        on_success,
        on_failure,
        timeout: preamble_timeout,
    } = config;

    let result = run_preamble_exchange(stream, &preamble, on_success, preamble_timeout).await;

    // On failure, invoke the failure callback if registered.
    if let Err(ref err) = result {
        invoke_failure_handler(stream, err, on_failure.as_ref()).await;
    }

    result
}

/// Execute the preamble write and success callback, with optional timeout.
async fn run_preamble_exchange<P>(
    stream: &mut TcpStream,
    preamble: &P,
    on_success: Option<ClientPreambleSuccessHandler<P>>,
    preamble_timeout: Option<Duration>,
) -> Result<Vec<u8>, ClientError>
where
    P: Encode + Send + Sync + 'static,
{
    let exchange = async {
        // Write the preamble.
        write_preamble(stream, preamble)
            .await
            .map_err(ClientError::PreambleEncode)?;

        // Invoke success callback if registered, collecting leftover bytes.
        match on_success.as_ref() {
            Some(handler) => handler(preamble, stream)
                .await
                .map_err(ClientError::PreambleRead),
            None => Ok(Vec::new()),
        }
    };

    // Apply timeout if configured.
    match preamble_timeout {
        Some(limit) => timeout(limit, exchange)
            .await
            .unwrap_or(Err(ClientError::PreambleTimeout)),
        None => exchange.await,
    }
}

/// Invoke the failure handler if one is registered.
async fn invoke_failure_handler(
    stream: &mut TcpStream,
    err: &ClientError,
    on_failure: Option<&ClientPreambleFailureHandler>,
) {
    if let Some(handler) = on_failure
        && let Err(e) = handler(err, stream).await
    {
        error!("preamble failure handler error: {e}");
    }
}
