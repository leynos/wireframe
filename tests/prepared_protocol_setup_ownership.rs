//! Protocol-ownership probe at the connection-setup boundary.
//!
//! #644 gives a connection exactly one strong reference to the installed
//! protocol, held by its `ProtocolHooks`. The dispatch-time probe in
//! `protocol_ownership.rs` measures that invariant while a handler runs,
//! after setup has already returned. This probe pauses inside
//! `on_connection_setup` and reads the live count from the test, so an extra
//! owned clone taken for setup fails here rather than passing unnoticed.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use tokio::time::timeout;
use wireframe::{
    app::{Envelope, Handler},
    hooks::{ConnectionContext, WireframeProtocol},
    push::PushHandle,
};
use wireframe_testing::TestResult;

/// Application type using the default length-delimited codec (`Bytes` frames).
type ResponseApp =
    wireframe::app::WireframeApp<wireframe::serializer::BincodeSerializer, (), Envelope>;

/// Type of the protocol reference a connection installs.
type InstalledProtocol = Arc<dyn WireframeProtocol<Frame = Bytes, ProtocolError = ()>>;

/// A handler that ignores every request; this probe only needs a live route.
fn no_op_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

/// Protocol that pauses inside setup so the test can read the live count.
struct PausingSetupProtocol {
    /// Signals the test that setup has started.
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    /// Released by the test once it has read the count.
    ///
    /// The protocol must be `Sync`, and a receiver is not, so the mutex both
    /// satisfies that bound and makes the single release explicit.
    resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl WireframeProtocol for PausingSetupProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {
        let _ = self.entered.send(());
        // Bounded so a scheduling mistake fails the assertion below instead of
        // hanging the suite. The test releases setup as soon as it has read
        // the count.
        let _ = self
            .resume
            .lock()
            .expect("resume lock")
            .recv_timeout(Duration::from_secs(5));
    }
}

/// Prepared setup sees the hooks' single protocol reference, not a clone.
///
/// Setup runs on the connection task while the test reads the count, so the
/// measurement covers the boundary where an extra owned clone would appear.
/// The runtime needs a second worker because setup blocks its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_setup_holds_one_protocol_reference() -> TestResult<()> {
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel::<()>();
    let app = ResponseApp::new()?
        .route(1, no_op_handler())?
        .with_protocol(PausingSetupProtocol {
            entered: entered_tx,
            resume: std::sync::Mutex::new(resume_rx),
        });
    let prepared = app
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared = Arc::new(prepared);
    let probe: InstalledProtocol = prepared.protocol().expect("protocol should be installed");
    assert_eq!(
        Arc::strong_count(&probe),
        2,
        "the prepared template holds one reference and this probe adds one"
    );

    let (client, server) = tokio::io::duplex(64);
    let connection = tokio::spawn({
        let prepared = Arc::clone(&prepared);
        async move { prepared.handle_connection_result(server).await }
    });

    entered_rx.recv().await.expect("setup should start");
    assert_eq!(
        Arc::strong_count(&probe),
        3,
        "setup holds the template's reference, this probe, and the connection's single hooks \
         reference"
    );
    resume_tx.send(()).expect("resume setup");

    drop(client);
    timeout(Duration::from_secs(2), connection)
        .await
        .expect("the connection should finish")?
        .expect("the connection should exit cleanly");
    assert_eq!(
        Arc::strong_count(&probe),
        2,
        "the finished connection released its single reference"
    );
    Ok(())
}
