//! Regression coverage for connection-runtime ordering guarantees.
//!
//! These tests pin the order in which the runtime releases the transport,
//! builds framing state, and runs the setup hook. They complement the
//! finalization tests in `tests.rs`.

use std::{
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, duplex},
    net::{TcpListener, TcpStream},
    sync::Notify,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use super::tests::drive;
use crate::{
    app::{Envelope, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    serializer::BincodeSerializer,
    test_helpers::CloneCountingCodec,
};

/// Teardown must observe a released transport, not the live framed stream.
///
/// The peer releases the teardown callback only after its read reports end of
/// stream. If the runtime still owned the framed transport while awaiting
/// teardown, neither side could make progress and the timeout would trip.
#[tokio::test]
async fn teardown_waits_only_after_the_transport_is_released() {
    let peer_eof = Arc::new(Notify::new());
    let teardown_wait = Arc::clone(&peer_eof);
    let teardowns = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::clone(&teardowns);

    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build application")
        .on_connection_setup(|| async {})
        .expect("register setup hook")
        .on_connection_teardown(move |()| {
            let gate = Arc::clone(&teardown_wait);
            let counter = Arc::clone(&teardown_count);
            Box::pin(async move {
                gate.notified().await;
                counter.fetch_add(1, Ordering::SeqCst);
            })
        })
        .expect("register teardown hook")
        .prepare()
        .await
        .expect("prepare application");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let address = listener.local_addr().expect("listener address");
    let mut client = TcpStream::connect(address).await.expect("connect client");
    let (server, _) = listener.accept().await.expect("accept server");

    let peer_gate = Arc::clone(&peer_eof);
    let peer = tokio::spawn(async move {
        let mut byte = [0_u8; 1];
        let read = client.read(&mut byte).await.expect("peer read");
        assert_eq!(read, 0, "the peer should observe the closed transport");
        peer_gate.notify_one();
    });

    let token = CancellationToken::new();
    token.cancel();
    timeout(
        Duration::from_secs(5),
        drive(&prepared, server, Some(token)),
    )
    .await
    .expect("teardown must not deadlock on a transport the runtime still owns")
    .expect("cancelled connection should finish");

    peer.await.expect("peer task should finish");
    assert_eq!(
        teardowns.load(Ordering::SeqCst),
        1,
        "teardown should run after the peer observed closure"
    );
}

/// Setup must complete before the connection codec is cloned for framing.
#[tokio::test]
async fn setup_completes_before_the_connection_codec_is_cloned() {
    let clones = Arc::new(AtomicUsize::new(0));
    let setup_clones = Arc::clone(&clones);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let teardown_observed = Arc::clone(&observed);

    let prepared = WireframeApp::<BincodeSerializer, usize, Envelope>::new()
        .expect("build application")
        .with_codec(CloneCountingCodec::new(
            LengthDelimitedFrameCodec::default(),
            Arc::clone(&clones),
        ))
        .on_connection_setup(move || {
            let setup_counter = Arc::clone(&setup_clones);
            async move { setup_counter.load(Ordering::SeqCst) }
        })
        .expect("register setup hook")
        .on_connection_teardown(move |clone_count: usize| {
            let receipt_log = Arc::clone(&teardown_observed);
            async move {
                receipt_log.lock().expect("receipt log").push(clone_count);
            }
        })
        .expect("register teardown hook")
        .prepare()
        .await
        .expect("prepare application");

    let (client, server) = duplex(64);
    drop(client);
    drive(&prepared, server, None)
        .await
        .expect("connection should finish");

    assert_eq!(
        *observed.lock().expect("receipt log"),
        vec![0],
        "setup must run before the codec is cloned for framing"
    );
    assert_eq!(
        clones.load(Ordering::SeqCst),
        1,
        "framing should clone the codec once after setup"
    );
}

/// A pending setup must not clone the codec or start framing.
#[tokio::test]
async fn pending_setup_defers_codec_cloning_until_it_completes() {
    let clones = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let teardown_observed = Arc::clone(&observed);
    let release = Arc::new(Notify::new());
    let release_setup = Arc::clone(&release);
    let started = Arc::new(Notify::new());
    let setup_started = Arc::clone(&started);

    let prepared = WireframeApp::<BincodeSerializer, &'static str, Envelope>::new()
        .expect("build application")
        .with_codec(CloneCountingCodec::new(
            LengthDelimitedFrameCodec::default(),
            Arc::clone(&clones),
        ))
        .on_connection_setup(move || {
            let started_gate = Arc::clone(&setup_started);
            let release_gate = Arc::clone(&release_setup);
            async move {
                started_gate.notify_one();
                release_gate.notified().await;
                "connection state"
            }
        })
        .expect("register setup hook")
        .on_connection_teardown(move |state: &'static str| {
            let receipt_log = Arc::clone(&teardown_observed);
            async move {
                receipt_log.lock().expect("receipt log").push(state);
            }
        })
        .expect("register teardown hook")
        .prepare()
        .await
        .expect("prepare application");

    let (client, server) = duplex(64);
    drop(client);
    let shared = Arc::new(prepared);
    let connection = tokio::spawn({
        let connection_app = Arc::clone(&shared);
        async move { drive(&connection_app, server, None).await }
    });

    timeout(Duration::from_secs(5), started.notified())
        .await
        .expect("setup should start");
    sleep(Duration::from_millis(20)).await;
    assert_eq!(
        clones.load(Ordering::SeqCst),
        0,
        "a pending setup must not clone the codec"
    );
    assert!(
        observed.lock().expect("receipt log").is_empty(),
        "teardown must wait for setup"
    );

    release.notify_one();
    connection
        .await
        .expect("join connection task")
        .expect("connection should finish");
    assert_eq!(
        clones.load(Ordering::SeqCst),
        1,
        "framing should clone the codec once after setup completes"
    );
    assert_eq!(
        *observed.lock().expect("receipt log"),
        vec!["connection state"],
        "teardown should receive the setup value"
    );
}

/// Connection state that is deliberately not `Clone`.
struct ConnectionState {
    /// Identity recorded when teardown receives the value.
    id: u32,
}

/// Cooperative cancellation must finalize the exact non-`Clone` setup value.
#[tokio::test]
async fn cancellation_finalizes_the_exact_non_clone_setup_value() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let teardown_observed = Arc::clone(&observed);

    let prepared = WireframeApp::<BincodeSerializer, ConnectionState, Envelope>::new()
        .expect("build application")
        .on_connection_setup(|| async { ConnectionState { id: 7 } })
        .expect("register setup hook")
        .on_connection_teardown(move |state: ConnectionState| {
            let receipt_log = Arc::clone(&teardown_observed);
            async move {
                receipt_log.lock().expect("receipt log").push(state.id);
            }
        })
        .expect("register teardown hook")
        .prepare()
        .await
        .expect("prepare application");

    let token = CancellationToken::new();
    token.cancel();
    let (client, server) = duplex(64);
    drive(&prepared, server, Some(token))
        .await
        .expect("cancelled connection should finish");
    drop(client);

    assert_eq!(
        *observed.lock().expect("receipt log"),
        vec![7],
        "teardown should receive the exact non-Clone setup value"
    );
}
