//! Gauge regressions for accepted-connection accounting.
//!
//! The accept loop registers a connection before spawning its task, so the
//! active-connection gauge must cover the preamble handshake, application
//! processing, teardown, and panic recovery. Each test drives one connection
//! on a current-thread runtime so the thread-local recorder sees every update.
//! A snapshot drains the recorder's gauge, so each reading is the change since
//! the previous snapshot rather than an absolute value.

use std::sync::Arc;

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};
use tokio_util::task::TaskTracker;

use super::{ConnectionTask, PreambleHooks, spawn_connection_task};
use crate::{
    app::{Envelope, WireframeApp},
    connection::ActiveConnection,
    preamble::write_preamble,
    server::test_util::TestPreamble,
};

/// A single-byte preamble prefix that the decoder cannot complete.
const PARTIAL_PREAMBLE: [u8; 1] = [0x00];

/// A preamble whose declared string length exceeds the decode budget.
///
/// The layout is a four-byte identifier followed by an eight-byte big-endian
/// length of `u64::MAX`, which bincode rejects before allocating.
const OVERSIZED_PREAMBLE: [u8; 12] = [
    0x00, 0x00, 0x00, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
];

/// Bind a loopback listener and return the connected client and server ends.
async fn connected_pair() -> std::io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let client = TcpStream::connect(addr).await?;
    let (stream, _) = listener.accept().await?;
    Ok((client, stream))
}

/// Prepare a default application shared by the gauge tests.
async fn prepared_default_app() -> std::io::Result<Arc<crate::app::PreparedApp>> {
    WireframeApp::<crate::serializer::BincodeSerializer, (), Envelope>::default()
        .prepare()
        .await
        .map(Arc::new)
        .map_err(std::io::Error::other)
}

/// Snapshot the active-connection gauge, draining the change it accumulated.
///
/// [`Snapshotter::snapshot`] swaps each gauge back to zero, so the reading is
/// the change since the previous snapshot; after an increment and its matching
/// decrement it reports `0.0`.
fn read_active_gauge(snapshotter: &Snapshotter) -> f64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .find_map(|(key, _, _, value)| {
            if key.key().name() != crate::metrics::CONNECTIONS_ACTIVE {
                return None;
            }
            match value {
                DebugValue::Gauge(reading) => Some(*reading),
                _ => None,
            }
        })
        .unwrap_or(0.0)
}

/// Drive `body` inside a local recorder on a current-thread runtime.
fn with_current_thread_recorder(
    recorder: &DebuggingRecorder,
    body: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    ::metrics::with_local_recorder(recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(body);
        Ok(())
    })
}

/// Preamble hooks that accept any handshake without extra callbacks.
///
/// The struct literal avoids `PreambleHooks::default()`, whose derived bound
/// would require every test preamble type to implement [`Default`].
fn no_preamble_hooks() -> PreambleHooks<TestPreamble> {
    PreambleHooks {
        on_success: None,
        on_failure: None,
        timeout: None,
    }
}

/// The gauge covers a connection that is still waiting for its preamble.
#[test]
fn gauge_covers_a_pending_preamble() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    with_current_thread_recorder(&recorder, async move {
        let (mut client, stream) = connected_pair().await.expect("connect test pair");
        let app = prepared_default_app().await.expect("prepare default app");
        let tracker = TaskTracker::new();
        spawn_connection_task(
            ConnectionTask {
                stream,
                app,
                hooks: no_preamble_hooks(),
                connection: ActiveConnection::new(),
            },
            &tracker,
        );

        tokio::task::yield_now().await;
        assert_eq!(
            read_active_gauge(&snapshotter),
            1.0,
            "a connection waiting for its preamble must be counted"
        );

        // Close the peer so the pending read fails and the task finishes.
        client.shutdown().await.expect("shut down the test client");
        drop(client);
        tracker.close();
        tracker.wait().await;

        assert_eq!(
            read_active_gauge(&snapshotter),
            -1.0,
            "the gauge must be released once the task ends"
        );
    })
    .expect("run the pending-preamble scenario");
}

/// The gauge is released when the preamble fails to decode.
#[test]
fn gauge_released_after_preamble_rejection() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    with_current_thread_recorder(&recorder, async move {
        let (mut client, stream) = connected_pair().await.expect("connect test pair");
        let app = prepared_default_app().await.expect("prepare default app");
        let tracker = TaskTracker::new();
        spawn_connection_task(
            ConnectionTask {
                stream,
                app,
                hooks: no_preamble_hooks(),
                connection: ActiveConnection::new(),
            },
            &tracker,
        );

        client
            .write_all(&OVERSIZED_PREAMBLE)
            .await
            .expect("write oversized preamble");
        client.shutdown().await.expect("shut down client");
        drop(client);
        tracker.close();
        tracker.wait().await;

        assert_eq!(
            read_active_gauge(&snapshotter),
            0.0,
            "a rejected preamble must release the gauge"
        );
    })
    .expect("run the preamble-rejection scenario");
}

/// The gauge is released when a fully handshaken connection closes.
#[test]
fn gauge_released_after_normal_close() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    with_current_thread_recorder(&recorder, async move {
        let (mut client, stream) = connected_pair().await.expect("connect test pair");
        let app = prepared_default_app().await.expect("prepare default app");
        let tracker = TaskTracker::new();
        spawn_connection_task(
            ConnectionTask {
                stream,
                app,
                hooks: no_preamble_hooks(),
                connection: ActiveConnection::new(),
            },
            &tracker,
        );

        write_preamble(
            &mut client,
            &TestPreamble {
                id: 7,
                message: String::from("hello"),
            },
        )
        .await
        .expect("write preamble");
        tokio::task::yield_now().await;
        assert_eq!(
            read_active_gauge(&snapshotter),
            1.0,
            "the handshaken connection must stay counted while it reads frames"
        );

        drop(client);
        tracker.close();
        tracker.wait().await;

        assert_eq!(
            read_active_gauge(&snapshotter),
            -1.0,
            "a clean connection close must release the gauge"
        );
    })
    .expect("run the normal-close scenario");
}

/// A panic while handling the connection still releases the gauge.
#[test]
fn gauge_released_when_connection_task_panics() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    with_current_thread_recorder(&recorder, async move {
        let (mut client, stream) = connected_pair().await.expect("connect test pair");
        let app = Arc::new(
            WireframeApp::<crate::serializer::BincodeSerializer, (), Envelope>::default()
                .on_connection_setup(|| async { panic!("connection setup panic") })
                .expect("install panicking setup callback")
                .prepare()
                .await
                .expect("prepare panicking app"),
        );
        let tracker = TaskTracker::new();
        spawn_connection_task(
            ConnectionTask {
                stream,
                app,
                hooks: PreambleHooks::<()>::default(),
                connection: ActiveConnection::new(),
            },
            &tracker,
        );

        // Unit preambles consume no bytes, but the reader still waits for one
        // byte before decoding, so send a spare byte to unblock the handshake.
        client
            .write_all(&PARTIAL_PREAMBLE)
            .await
            .expect("write handshake byte");
        tracker.close();
        tracker.wait().await;

        assert_eq!(
            read_active_gauge(&snapshotter),
            0.0,
            "a panicking connection task must release the gauge"
        );
    })
    .expect("run the panicking-connection scenario");
}
