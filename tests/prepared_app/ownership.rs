//! Ownership proofs for the prepared application root.
//!
//! Each test installs a probed serializer, message assembler, lifecycle
//! callbacks, handler, and protocol, then asserts that:
//!
//! - nothing is dropped while the shared root or a live connection future still needs it;
//! - every probe drops exactly once when the final owner is released;
//! - two overlapping connections reach one assembler and one serializer allocation rather than
//!   per-connection clones.

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
    sync::{Notify, mpsc},
    time::{sleep, timeout},
};
use wireframe::{
    app::{Envelope, Handler, PreparedApp, WireframeApp},
    message_assembler::{FrameSequence, MessageKey},
    serializer::BincodeSerializer,
    test_helpers::{continuation_frame_payload, first_frame_payload},
};
use wireframe_testing::{TestResult, drive_prepared_with_frames};

use super::{
    build_frame,
    fallible_check_equal::check_equal,
    lifecycle_support::{OwnedToken, Recorder},
    ownership_support::{
        CallRecorder,
        DropLedger,
        DropProbe,
        ProbeAssembler,
        ProbeProtocol,
        ProbeSerializer,
        probe_handler,
        sorted,
    },
};

/// Identity of the serializer probe.
const SERIALIZER_ID: u64 = 100;
/// Identity of the message assembler probe.
const ASSEMBLER_ID: u64 = 101;
/// Identity of the setup callback probe.
const SETUP_ID: u64 = 102;
/// Identity of the teardown callback probe.
const TEARDOWN_ID: u64 = 103;
/// Identity of the handler probe.
const HANDLER_ID: u64 = 104;
/// Identity of the protocol probe.
const PROTOCOL_ID: u64 = 105;

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

/// Wait until a recorder has observed at least `expected` calls.
async fn wait_for_calls(recorder: &CallRecorder, expected: usize) -> TestResult<()> {
    timeout(Duration::from_secs(2), async {
        while recorder.calls() < expected {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| format!("probe was not called {expected} times"))?;
    Ok(())
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

    entered.notified().await;
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

/// Two clients' pipes while they interleave one message: A then B, B then A.
struct Interleaved {
    /// Client side of the first connection.
    client_a: tokio::io::DuplexStream,
    /// Client side of the second connection.
    client_b: tokio::io::DuplexStream,
    /// Assembled payloads in the order the handler delivered them.
    received: mpsc::UnboundedReceiver<Vec<u8>>,
    /// Task driving the first connection.
    task_a: tokio::task::JoinHandle<()>,
    /// Task driving the second connection.
    task_b: tokio::task::JoinHandle<()>,
}

impl Interleaved {
    /// Drive both connections through one complete two-fragment message each.
    ///
    /// The fragments alternate deliberately — A's first, B's first, A's last,
    /// B's last — so each connection's reassembly state must be its own while
    /// both connections reach the one prepared assembler.
    async fn exchange_fragments(
        &mut self,
        assembler: &CallRecorder,
    ) -> TestResult<(Vec<u8>, Vec<u8>)> {
        let key = MessageKey::from(9);
        let sequence = FrameSequence(1);

        self.client_a
            .write_all(&build_frame(
                1,
                first_frame_payload(key, b"one", false, Some(6))?,
            )?)
            .await?;
        wait_for_calls(assembler, 1).await?;
        self.client_b
            .write_all(&build_frame(
                1,
                first_frame_payload(key, b"red", false, Some(6))?,
            )?)
            .await?;
        wait_for_calls(assembler, 2).await?;

        self.client_a
            .write_all(&build_frame(
                1,
                continuation_frame_payload(key, sequence, b"two", true)?,
            )?)
            .await?;
        let payload_a = recv_payload(&mut self.received).await?;
        self.client_b
            .write_all(&build_frame(
                1,
                continuation_frame_payload(key, sequence, b"blu", true)?,
            )?)
            .await?;
        let payload_b = recv_payload(&mut self.received).await?;
        Ok((payload_a, payload_b))
    }

    /// Close both clients and await both connection tasks.
    async fn finish(self) -> TestResult<()> {
        let Self {
            mut client_a,
            mut client_b,
            task_a,
            task_b,
            ..
        } = self;
        client_a.shutdown().await?;
        client_b.shutdown().await?;
        finish_connection(task_a).await?;
        finish_connection(task_b).await
    }
}

/// Two connections interleave the same message key on one assembler.
#[tokio::test]
async fn overlapping_connections_share_one_assembler_and_serializer() -> TestResult<()> {
    let ledger = DropLedger::default();
    let (payload_sender, received) = mpsc::unbounded_channel::<Vec<u8>>();
    let handler: Handler<Envelope> = Arc::new(move |envelope: &Envelope| {
        let payload = envelope.payload_bytes().to_vec();
        let reply = payload_sender.clone();
        Box::pin(async move {
            drop(reply.send(payload));
        })
    });

    let serializer = ProbeSerializer::new(SERIALIZER_ID, ledger.clone());
    let assembler = ProbeAssembler::new(ASSEMBLER_ID, ledger.clone());
    let serializer_recorder = serializer.recorder();
    let assembler_recorder = assembler.recorder();
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .serializer(serializer)
        .with_message_assembler(assembler)
        .route(1, handler)?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn Error + Send + Sync> { Box::new(error) })?;
    let root = Arc::new(prepared);

    let (client_a, server_a) = duplex(4096);
    let (client_b, server_b) = duplex(4096);
    let task_a = spawn_connection(Arc::clone(&root), server_a);
    let task_b = spawn_connection(Arc::clone(&root), server_b);
    let mut interleaved = Interleaved {
        client_a,
        client_b,
        received,
        task_a,
        task_b,
    };
    let (payload_a, payload_b) = interleaved.exchange_fragments(&assembler_recorder).await?;
    interleaved.finish().await?;

    check_equal(
        &payload_a,
        &b"onetwo".to_vec(),
        "connection A assembled its message",
    )?;
    check_equal(
        &payload_b,
        &b"redblu".to_vec(),
        "connection B assembled its message",
    )?;
    let parsed_all_fragments = assembler_recorder.calls() >= 4;
    check_equal(
        &parsed_all_fragments,
        &true,
        "both connections should parse their first and final fragments",
    )?;
    let one_assembler = assembler_recorder.single_address().is_some();
    check_equal(
        &one_assembler,
        &true,
        "every connection should reach the one prepared assembler",
    )?;
    let one_serializer = serializer_recorder.single_address().is_some();
    check_equal(
        &one_serializer,
        &true,
        "every connection should reach the one prepared serializer",
    )?;

    drop(root);
    check_equal(
        &sorted(ledger.dropped()),
        &sorted(vec![SERIALIZER_ID, ASSEMBLER_ID]),
        "dropping the root should release the serializer and assembler once",
    )?;
    Ok(())
}

/// Spawn one connection task that owns a clone of the shared root.
fn spawn_connection<C>(
    root: Arc<PreparedApp<ProbeSerializer, C, Envelope>>,
    stream: tokio::io::DuplexStream,
) -> tokio::task::JoinHandle<()>
where
    C: Send + 'static,
{
    tokio::spawn(async move {
        root.handle_connection(stream).await;
    })
}

/// Receive one assembled payload with a bounded wait.
async fn recv_payload(receiver: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> TestResult<Vec<u8>> {
    timeout(Duration::from_secs(2), receiver.recv())
        .await
        .map_err(|_| "assembled payload did not arrive")?
        .ok_or_else(|| "handler channel closed".into())
}

/// Await a connection task and surface a panic as a test error.
async fn finish_connection(task: tokio::task::JoinHandle<()>) -> TestResult<()> {
    timeout(Duration::from_secs(2), task)
        .await
        .map_err(|_| "connection task did not finish")?
        .map_err(|error| -> Box<dyn Error + Send + Sync> { Box::new(error) })?;
    Ok(())
}
