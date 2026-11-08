//! Unit coverage for connection-local runtime finalization.

use std::{
    io,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, duplex},
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use super::finalizer::ConnectionFinalizer;
use crate::{
    app::{
        Envelope,
        PreparedApp,
        WireframeApp,
        inbound_handler::{ConnectionProcessingContext, process_connection_with_cancellation},
        lifecycle::ConnectionTeardown,
    },
    codec::FrameCodec,
    serializer::BincodeSerializer,
    test_helpers::{BadCodec, counting_teardown},
};

/// Prepared application whose connection state is the unit type.
type TestPrepared = PreparedApp<BincodeSerializer, (), Envelope>;

/// Build an application that increments `calls` from its teardown hook.
///
/// Registration failures are programming errors in this fixture, so panicking
/// with context keeps the connection-level assertions readable.
fn app_with_teardown(calls: Arc<AtomicUsize>) -> WireframeApp {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build application")
        .on_connection_setup(|| async {})
        .expect("register setup hook")
        .on_connection_teardown(counting_teardown(calls))
        .expect("register teardown hook")
}

/// Prepare [`app_with_teardown`] for connection-level driving.
async fn prepared_with_teardown(calls: Arc<AtomicUsize>) -> TestPrepared {
    app_with_teardown(calls)
        .prepare()
        .await
        .expect("prepare application")
}

/// Borrow the prepared application as the runtime's processing context.
fn context<C, F>(
    prepared: &PreparedApp<BincodeSerializer, C, Envelope, F>,
) -> ConnectionProcessingContext<'_, BincodeSerializer, C, Envelope, F>
where
    C: Send + 'static,
    F: crate::codec::FrameCodec,
{
    ConnectionProcessingContext {
        routes: &prepared.routes,
        serializer: &prepared.serializer,
        codec: &prepared.codec,
        on_connect: prepared.on_connect.as_ref(),
        on_disconnect: prepared.on_disconnect.as_ref(),
        message_assembler: prepared.message_assembler.as_ref(),
        fragmentation: prepared.fragmentation,
        protocol_hooks: prepared.protocol_hooks(),
        memory_budgets: prepared.memory_budgets,
        read_timeout_ms: prepared.read_timeout_ms,
    }
}

/// Drive one connection through the cancellation-aware entrypoint.
async fn drive<C, F, W>(
    prepared: &PreparedApp<BincodeSerializer, C, Envelope, F>,
    stream: W,
    cancellation: Option<CancellationToken>,
) -> io::Result<()>
where
    C: Send + 'static,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    process_connection_with_cancellation(stream, context(prepared), cancellation).await
}

/// Cancellation requested before the first read still finalizes exactly once.
#[tokio::test]
async fn cancellation_before_any_frame_runs_teardown_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let prepared = prepared_with_teardown(Arc::clone(&calls)).await;
    let token = CancellationToken::new();
    token.cancel();
    let (client, server) = duplex(64);

    drive(&prepared, server, Some(token))
        .await
        .expect("cancelled connection should finish");
    drop(client);

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "cancellation should finalize exactly once"
    );
}

/// Cancellation observed while a read is pending still finalizes exactly once.
#[tokio::test]
async fn cancellation_during_pending_read_runs_teardown_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let prepared = prepared_with_teardown(Arc::clone(&calls)).await;
    let token = CancellationToken::new();
    let canceller = token.clone();
    let (client, server) = duplex(64);

    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(20)).await;
        canceller.cancel();
    });
    let result = timeout(
        Duration::from_secs(5),
        drive(&prepared, server, Some(token)),
    )
    .await;
    cancel_task.await.expect("canceller task should finish");
    drop(client);

    result
        .expect("cancellation should end the connection promptly")
        .expect("cancelled connection should finish without an error");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "pending-read cancellation should finalize exactly once"
    );
}

/// A decoder rejection runs teardown exactly once before returning the error.
#[tokio::test]
async fn decode_rejection_runs_teardown_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build application")
        .with_codec(BadCodec)
        .on_connection_setup(|| async {})
        .expect("register setup hook")
        .on_connection_teardown(counting_teardown(Arc::clone(&calls)))
        .expect("register teardown hook")
        .prepare()
        .await
        .expect("prepare application");
    let (mut client, server) = duplex(64);
    client.write_all(&[1]).await.expect("write malformed frame");
    client.shutdown().await.expect("close client");

    let error = drive(&prepared, server, None)
        .await
        .expect_err("bad codec should reject the frame");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "decode rejection should finalize exactly once"
    );
}

/// The finalizer takes its state at most once across every transition.
#[test]
fn finalizer_transitions_take_state_at_most_once() {
    let mut armed = ConnectionFinalizer::<u32>::not_set_up();
    armed.arm(7);
    assert_eq!(armed.take_for_teardown(), Some(7));
    assert_eq!(armed.take_for_teardown(), None);
    armed.arm(9);
    assert_eq!(
        armed.take_for_teardown(),
        None,
        "arming a finalized finalizer must be ignored"
    );

    // Taking before setup succeeds also finalizes the value, so a later arm is
    // ignored rather than resurrecting a teardown that was already skipped.
    let mut abandoned = ConnectionFinalizer::<u32>::not_set_up();
    assert_eq!(abandoned.take_for_teardown(), None);
    abandoned.arm(11);
    assert_eq!(abandoned.take_for_teardown(), None);
}

/// Arming twice keeps the first state and drops the second.
#[test]
fn finalizer_arming_twice_keeps_the_first_state() {
    let mut finalizer = ConnectionFinalizer::not_set_up();
    finalizer.arm(1);
    finalizer.arm(2);
    assert_eq!(finalizer.take_for_teardown(), Some(1));
}

/// Finalizing without setup state never invokes the teardown callback.
#[tokio::test]
async fn finalizer_skips_teardown_without_setup_state() {
    let calls = Arc::new(AtomicUsize::new(0));
    let teardown = shared_counting_teardown(Arc::clone(&calls));
    let mut finalizer = ConnectionFinalizer::<u32>::not_set_up();

    finalizer.finalize(Some(&teardown)).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Finalizing an armed finalizer invokes teardown once and never again.
#[tokio::test]
async fn finalizer_invokes_teardown_once() {
    let states = Arc::new(Mutex::new(Vec::new()));
    let teardown = recording_teardown(Arc::clone(&states));
    let mut finalizer = ConnectionFinalizer::not_set_up();
    finalizer.arm(5);

    finalizer.finalize(Some(&teardown)).await;
    finalizer.finalize(Some(&teardown)).await;

    assert_eq!(
        *states.lock().expect("state log mutex"),
        vec![5],
        "teardown should receive the armed state exactly once"
    );
}

/// Build a shared counting callback for driving a finalizer directly.
fn shared_counting_teardown(calls: Arc<AtomicUsize>) -> Arc<ConnectionTeardown<u32>> {
    Arc::new(move |_state: u32| {
        let counter = Arc::clone(&calls);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    })
}

/// Build a teardown callback that records the state it receives.
fn recording_teardown(states: Arc<Mutex<Vec<u32>>>) -> Arc<ConnectionTeardown<u32>> {
    Arc::new(move |state| {
        let state_log = Arc::clone(&states);
        Box::pin(async move {
            state_log.lock().expect("state log mutex").push(state);
        })
    })
}
