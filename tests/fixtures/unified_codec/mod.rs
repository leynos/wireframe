//! `UnifiedCodecWorld` fixture for rstest-bdd tests.
//!
//! Validates that all outbound frames pass through the [`FramePipeline`]
//! before reaching the wire, exercising the unified codec path end-to-end
//! via `WireframeApp::handle_connection_result` over in-memory duplex
//! streams.

mod transport;

use std::io;

use rstest::fixture;
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Handler, Packet, WireframeApp},
    fragment::FragmentationConfig,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// Default route identifier used in unified codec tests.
const ROUTE_ID: u32 = 42;
/// Default correlation identifier used in unified codec tests.
const CORRELATION: Option<u64> = Some(7);

/// Test world for the unified codec pipeline BDD scenarios.
#[derive(Debug)]
pub struct UnifiedCodecWorld {
    pub(super) capacity: usize,
    pub(super) fragmentation: Option<FragmentationConfig>,
    pub(super) client: Option<Framed<tokio::io::DuplexStream, LengthDelimitedCodec>>,
    pub(super) server_handle: Option<JoinHandle<io::Result<()>>>,
    pub(super) handler_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    pub(super) sent_payloads: Vec<Vec<u8>>,
    /// Handler-observed payloads, in order.
    pub handler_observed: Vec<Vec<u8>>,
    pub(super) response_payloads: Vec<Vec<u8>>,
    pub(super) last_response_fragmented: Option<bool>,
}

impl Default for UnifiedCodecWorld {
    fn default() -> Self {
        Self {
            capacity: 512,
            fragmentation: None,
            client: None,
            server_handle: None,
            handler_rx: None,
            sent_payloads: Vec::new(),
            handler_observed: Vec::new(),
            response_payloads: Vec::new(),
            last_response_fragmented: None,
        }
    }
}

/// Fixture for unified codec pipeline scenarios used by rstest-bdd steps.
///
/// Note: `rustfmt::skip` prevents single-line collapse that triggers
/// `unused_braces`.
#[rustfmt::skip]
#[fixture]
pub fn unified_codec_world() -> UnifiedCodecWorld {
    UnifiedCodecWorld::default()
}

impl UnifiedCodecWorld {
    /// Start an echo server with optional fragmentation.
    ///
    /// # Errors
    /// Returns an error if app creation or spawning fails.
    pub fn start_server(
        &mut self,
        runtime: &Runtime,
        capacity: usize,
        fragmentation: bool,
    ) -> TestResult {
        self.capacity = capacity;

        let (tx, rx) = mpsc::unbounded_channel();
        self.handler_rx = Some(rx);

        let handler = Self::make_handler(&tx);
        let mut app: WireframeApp = WireframeApp::new()?.buffer_capacity(capacity);

        if fragmentation {
            let config = Self::fragmentation_config(capacity)?;
            self.fragmentation = Some(config);
            app = app.fragmentation(Some(config));
        }

        let app = app.route(ROUTE_ID, handler)?;
        let codec = app.length_codec();
        let (client_stream, server_stream) = tokio::io::duplex(256);
        let client = Framed::new(client_stream, codec.clone());
        let server =
            runtime.spawn(async move { app.handle_connection_result(server_stream).await });

        self.client = Some(client);
        self.server_handle = Some(server);
        Ok(())
    }

    /// Verify that all handler-observed payloads match the sent payloads.
    ///
    /// # Errors
    /// Returns an error if payloads do not match.
    pub fn verify_handler_payloads(&self) -> TestResult {
        self.verify_payloads_match(&self.handler_observed, "handler")
    }

    /// Verify that all response payloads match the sent payloads.
    ///
    /// # Errors
    /// Returns an error if payloads do not match.
    pub fn verify_response_payloads(&self) -> TestResult {
        self.verify_payloads_match(&self.response_payloads, "response")
    }

    /// Verify the last response was not fragmented.
    ///
    /// # Errors
    /// Returns an error if the response was fragmented.
    pub fn verify_unfragmented(&self) -> TestResult {
        match self.last_response_fragmented {
            Some(false) => Ok(()),
            Some(true) => Err("expected unfragmented response".into()),
            None => Err("no response collected yet".into()),
        }
    }

    /// Await server shutdown.
    ///
    /// # Errors
    /// Returns an error if the server task panicked or returned an error.
    pub async fn await_server(&mut self) -> TestResult {
        if let Some(handle) = self.server_handle.take() {
            handle.await??;
        }
        Ok(())
    }

    fn verify_payloads_match(&self, observed: &[Vec<u8>], label: &str) -> TestResult {
        if observed.len() != self.sent_payloads.len() {
            return Err(format!(
                "{label} payload count mismatch: expected {}, got {}",
                self.sent_payloads.len(),
                observed.len()
            )
            .into());
        }
        for (i, (observed, expected)) in observed.iter().zip(self.sent_payloads.iter()).enumerate()
        {
            if observed != expected {
                return Err(format!("{label} payload {i} mismatch").into());
            }
        }
        Ok(())
    }

    fn make_handler(sender: &mpsc::UnboundedSender<Vec<u8>>) -> Handler<Envelope> {
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
}
