//! Ownership proofs for the prepared application root.
//!
//! Each test installs a probed serializer, message assembler, lifecycle
//! callbacks, handler, and protocol, then asserts that:
//!
//! - nothing is dropped while the shared root or a live connection future still needs it;
//! - every probe drops exactly once when the final owner is released.

use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncWriteExt, duplex},
    sync::Notify,
    time::timeout,
};
use wireframe::{
    app::{Envelope, Handler, PreparedApp, WireframeApp},
    message_assembler::MessageKey,
    serializer::BincodeSerializer,
    test_helpers::first_frame_payload,
};
use wireframe_testing::{TestResult, drive_prepared_with_frames};

use super::{
    build_frame,
    fallible_check_equal::check_equal,
    lifecycle_support::{OwnedToken, Recorder},
    ownership_support::{
        ASSEMBLER_ID,
        DropLedger,
        DropProbe,
        HANDLER_ID,
        PROTOCOL_ID,
        ProbeAssembler,
        ProbeProtocol,
        ProbeSerializer,
        SERIALIZER_ID,
        SETUP_ID,
        TEARDOWN_ID,
        probe_handler,
        sorted,
    },
};

/// Connection state produced by the probed setup callback.
type ProbePrepared = PreparedApp<ProbeSerializer, OwnedToken, Envelope>;

/// Build an application whose lifecycle callbacks and protocol are probed.
///
/// The handler is supplied by the caller so a test can control whether the
/// route pauses, counts invocations, or observes a drop.
async fn build_probed_app(
    ledger: &DropLedger,
    tokens: &Recorder,
    next_token: &Arc<AtomicU64>,
    handler: Handler<Envelope>,
) -> TestResult<ProbePrepared> {
    let serializer = ProbeSerializer::new(SERIALIZER_ID, ledger.clone());
    let assembler = ProbeAssembler::new(ASSEMBLER_ID, ledger.clone());

    let setup_probe = DropProbe::new(ledger.clone(), SETUP_ID);
    let teardown_probe = DropProbe::new(ledger.clone(), TEARDOWN_ID);
    let setup_tokens = tokens.clone();
    let setup_next = Arc::clone(next_token);
    let teardown_tokens = tokens.clone();

    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .serializer(serializer)
        .with_message_assembler(assembler)
        .with_protocol(ProbeProtocol::new(
            PROTOCOL_ID,
            ledger.clone(),
            Arc::new(AtomicUsize::new(0)),
        ))
        .route(1, handler)?
        .on_connection_setup(move || {
            let _keep_alive = setup_probe.id();
            let recorder = setup_tokens.clone();
            let id = setup_next.fetch_add(1, Ordering::SeqCst);
            async move { OwnedToken::create(recorder, id) }
        })?
        .on_connection_teardown(move |token: OwnedToken| {
            let _keep_alive = teardown_probe.id();
            let recorder = teardown_tokens.clone();
            async move {
                recorder.record_receipt(token.id());
                drop(token);
            }
        })?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn Error + Send + Sync> { Box::new(error) })?;

    Ok(prepared)
}

/// Every application-owned probe identity used by [`build_probed_app`].
fn all_probe_ids() -> Vec<u64> {
    vec![
        SERIALIZER_ID,
        ASSEMBLER_ID,
        SETUP_ID,
        TEARDOWN_ID,
        PROTOCOL_ID,
    ]
}

/// A live connection future and a lingering root clone keep every probe alive.
#[tokio::test]
async fn connection_and_root_clone_keep_every_probe_alive() -> TestResult<()> {
    let ledger = DropLedger::default();
    let tokens = Recorder::default();
    let next_token = Arc::new(AtomicU64::new(1));

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let handler_gate = Arc::clone(&entered);
    let handler_release = Arc::clone(&release);
    let handler: Handler<Envelope> = Arc::new(move |_envelope: &Envelope| {
        let call_entered = Arc::clone(&handler_gate);
        let call_release = Arc::clone(&handler_release);
        Box::pin(async move {
            call_entered.notify_one();
            call_release.notified().await;
        })
    });

    let prepared = build_probed_app(&ledger, &tokens, &next_token, handler).await?;
    let root = Arc::new(prepared);
    let connection_root = Arc::clone(&root);
    // A complete single-fragment message, because the installed assembler
    // parses every inbound payload as a fragment header; a plain envelope
    // payload would be rejected before dispatch and the gated handler would
    // never run.
    let frame = build_frame(
        1,
        first_frame_payload(MessageKey::from(7), b"hello", true, None)?,
    )?;
    let connection = tokio::spawn(async move {
        let (mut client, server) = duplex(1024);
        client.write_all(&frame).await.expect("write request");
        client.shutdown().await.expect("close request stream");
        drop(client);
        connection_root.handle_connection(server).await;
    });

    // Bound the wait: a connection task that stops before dispatch would never
    // notify, and an unbounded wait would hang the test rather than report it.
    let entered_result = timeout(Duration::from_secs(2), entered.notified()).await;
    if entered_result.is_err() {
        connection.abort();
        let joined = connection.await;
        return Err(
            format!("handler was not entered before the connection ended: {joined:?}").into(),
        );
    }
    check_equal(
        &ledger.is_empty(),
        &true,
        "no application-owned probe should drop while the connection runs",
    )?;
    check_equal(
        &tokens.dropped_ids().is_empty(),
        &true,
        "connection state should stay alive while the handler runs",
    )?;

    release.notify_one();
    timeout(Duration::from_secs(5), connection)
        .await
        .map_err(|_| "connection did not finish")?
        .expect("connection task should not panic");

    let created = tokens.created_ids();
    check_equal(
        &tokens.dropped_ids(),
        &created,
        "teardown should drop exactly the state setup created",
    )?;
    check_equal(
        &ledger.is_empty(),
        &true,
        "dropping the connection future must not drop the shared root's values",
    )?;

    check_final_root_drop(&ledger, root)
}

/// Drop every root clone and assert each probe releases exactly once.
///
/// A second clone is held across the first drop, so a probe released by the
/// connection rather than by the root would be caught before the last drop.
fn check_final_root_drop(ledger: &DropLedger, root: Arc<ProbePrepared>) -> TestResult<()> {
    let lingering = Arc::clone(&root);
    drop(root);
    check_equal(
        &ledger.is_empty(),
        &true,
        "a surviving root clone should keep every probe alive",
    )?;
    drop(lingering);
    check_equal(
        &sorted(ledger.dropped()),
        &sorted(all_probe_ids()),
        "the final root drop should release every probe exactly once",
    )?;
    Ok(())
}

/// The handler allocation is shared by two route ids and never cloned again.
#[tokio::test]
async fn one_handler_allocation_serves_two_route_ids() -> TestResult<()> {
    let ledger = DropLedger::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = probe_handler(ledger.clone(), HANDLER_ID, Arc::clone(&calls));
    let weak = Arc::downgrade(&handler);
    let external = Arc::clone(&handler);

    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .route(1, Arc::clone(&handler))?
        .route(2, handler)?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn Error + Send + Sync> { Box::new(error) })?;
    check_equal(
        &weak.strong_count(),
        &3usize,
        "two route entries plus the external owner should share one allocation",
    )?;

    let first =
        drive_prepared_with_frames(&prepared, vec![build_frame(1, b"one".to_vec())?]).await?;
    let second =
        drive_prepared_with_frames(&prepared, vec![build_frame(2, b"two".to_vec())?]).await?;
    check_equal(&first.is_empty(), &false, "route 1 should respond")?;
    check_equal(&second.is_empty(), &false, "route 2 should respond")?;
    check_equal(
        &calls.load(Ordering::SeqCst),
        &2usize,
        "both route ids should invoke the same handler allocation",
    )?;

    drop(prepared);
    check_equal(
        &weak.strong_count(),
        &1usize,
        "preparation should not leave a clone behind",
    )?;
    check_equal(
        &ledger.is_empty(),
        &true,
        "the externally owned handler should keep the probe alive",
    )?;
    drop(external);
    check_equal(
        &ledger.dropped(),
        &vec![HANDLER_ID],
        "the handler probe should drop once after the last owner",
    )?;
    Ok(())
}

/// A protocol handle obtained from the prepared root outlives that root.
#[tokio::test]
async fn protocol_handle_outlives_the_prepared_root() -> TestResult<()> {
    let ledger = DropLedger::default();
    let setups = Arc::new(AtomicUsize::new(0));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_protocol(ProbeProtocol::new(
            PROTOCOL_ID,
            ledger.clone(),
            Arc::clone(&setups),
        ))
        .prepare()
        .await
        .map_err(|error| -> Box<dyn Error + Send + Sync> { Box::new(error) })?;

    let (mut client, server) = duplex(64);
    client.shutdown().await?;
    prepared.handle_connection(server).await;
    check_equal(
        &setups.load(Ordering::SeqCst),
        &1usize,
        "the protocol should observe one connection setup",
    )?;

    let handle = prepared.protocol().ok_or("protocol should be installed")?;
    drop(prepared);
    check_equal(
        &ledger.is_empty(),
        &true,
        "the cloned protocol handle should keep the probe alive",
    )?;
    drop(handle);
    check_equal(
        &ledger.dropped(),
        &vec![PROTOCOL_ID],
        "the protocol probe should drop once after its last handle",
    )?;
    Ok(())
}
