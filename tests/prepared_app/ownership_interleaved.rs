//! Cross-connection sharing proofs for the prepared application root.
//!
//! Two connections interleave the fragments of one message key, so each
//! connection's reassembly state must stay its own while both connections
//! reach the one prepared serializer and the one prepared assembler.

use std::{
    error::Error,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncWriteExt, duplex},
    sync::mpsc,
    time::{sleep, timeout},
};
use wireframe::{
    app::{Envelope, Handler, PreparedApp, WireframeApp},
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    message_assembler::{FrameSequence, MessageKey},
    serializer::{BincodeSerializer, Serializer},
    test_helpers::{continuation_frame_payload, first_frame_payload},
};
use wireframe_testing::TestResult;

use super::{
    build_frame,
    fallible_check_equal::check_equal,
    ownership_support::{
        ASSEMBLER_ID,
        CallRecorder,
        DropLedger,
        ProbeAssembler,
        ProbeSerializer,
        SERIALIZER_ID,
        sorted,
    },
};

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

/// One prepared root plus the recorders that observe its shared components.
struct ProbedRoot {
    /// Shared root both connections are driven from.
    root: Arc<PreparedApp<ProbeSerializer, (), Envelope>>,
    /// Recorder attached to the one prepared serializer.
    serializer_recorder: CallRecorder,
    /// Recorder attached to the one prepared assembler.
    assembler_recorder: CallRecorder,
}

/// Build one prepared root whose serializer and assembler are probes.
async fn prepare_probed_root(
    handler: Handler<Envelope>,
    ledger: &DropLedger,
) -> TestResult<ProbedRoot> {
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
    Ok(ProbedRoot {
        root: Arc::new(prepared),
        serializer_recorder,
        assembler_recorder,
    })
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

    let ProbedRoot {
        root,
        serializer_recorder,
        assembler_recorder,
    } = prepare_probed_root(handler, &ledger).await?;

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

/// Per-connection state identified so its teardown can be matched.
struct ConnectionState {
    /// Identifier assigned by the setup hook.
    id: u64,
}

/// Handler that ignores every request; this test only needs live routes.
fn no_op_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

/// Box a preparation or lifecycle error for the test result.
fn boxed_error(error: impl Error + Send + Sync + 'static) -> Box<dyn Error + Send + Sync> {
    Box::new(error)
}

/// Snapshot a lifecycle log.
fn events(log: &Arc<Mutex<Vec<u64>>>) -> Vec<u64> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Wait until a lifecycle log holds at least `expected` entries.
async fn wait_for_events(log: &Arc<Mutex<Vec<u64>>>, expected: usize) -> TestResult<()> {
    timeout(Duration::from_secs(2), async {
        while events(log).len() < expected {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "expected {expected} lifecycle events, saw {:?}",
            events(log)
        )
    })?;
    Ok(())
}

/// Build the setup hook that gives each connection its own state.
///
/// The identifier is assigned when setup runs, and the same call records it so
/// both setups can be observed before either client closes.
fn connection_setup(
    next_id: Arc<AtomicU64>,
    setups: Arc<Mutex<Vec<u64>>>,
) -> impl Fn() -> std::future::Ready<ConnectionState> + Send + Sync + 'static {
    move || {
        let id = next_id.fetch_add(1, Ordering::SeqCst);
        setups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id);
        std::future::ready(ConnectionState { id })
    }
}

/// Build the teardown hook that records the state it was given.
fn connection_teardown(
    teardowns: Arc<Mutex<Vec<u64>>>,
) -> impl Fn(ConnectionState) -> std::future::Ready<()> + Send + Sync + 'static {
    move |state| {
        teardowns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(state.id);
        std::future::ready(())
    }
}

/// Two live connections receive distinct setup state, each torn down once.
///
/// Both setup hooks finish before either client closes, so the two
/// connections and their lifecycle states genuinely overlap. Teardown for
/// each connection must then observe that connection's own state exactly
/// once, rather than sharing one state or running a teardown twice.
#[tokio::test]
async fn overlapping_connections_receive_distinct_setup_state() -> TestResult<()> {
    let setups = Arc::new(Mutex::new(Vec::new()));
    let teardowns = Arc::new(Mutex::new(Vec::new()));
    let next_id = Arc::new(AtomicU64::new(1));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .on_connection_setup(connection_setup(Arc::clone(&next_id), Arc::clone(&setups)))
        .map_err(boxed_error)?
        .on_connection_teardown(connection_teardown(Arc::clone(&teardowns)))
        .map_err(boxed_error)?
        .route(1, no_op_handler())?
        .prepare()
        .await
        .map_err(boxed_error)?;
    let root = Arc::new(prepared);

    let (mut client_a, server_a) = duplex(64);
    let (mut client_b, server_b) = duplex(64);
    let task_a = spawn_connection(Arc::clone(&root), server_a);
    let task_b = spawn_connection(Arc::clone(&root), server_b);

    wait_for_events(&setups, 2).await?;
    let setup_ids = events(&setups);
    let mut distinct_setup_ids = sorted(setup_ids.clone());
    distinct_setup_ids.dedup();
    check_equal(
        &distinct_setup_ids.len(),
        &setup_ids.len(),
        "each connection received its own setup state",
    )?;

    client_a.shutdown().await?;
    finish_connection(task_a).await?;
    wait_for_events(&teardowns, 1).await?;
    client_b.shutdown().await?;
    finish_connection(task_b).await?;
    wait_for_events(&teardowns, 2).await?;

    let teardown_ids = events(&teardowns);
    check_equal(
        &sorted(teardown_ids.clone()),
        &sorted(setup_ids),
        "teardown received each setup state exactly once",
    )?;
    check_equal(&teardown_ids.len(), &2, "no connection tore down twice")?;
    Ok(())
}

/// Spawn one connection task that owns a clone of the shared root.
fn spawn_connection<S, C>(
    root: Arc<PreparedApp<S, C, Envelope>>,
    stream: tokio::io::DuplexStream,
) -> tokio::task::JoinHandle<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    Envelope: DecodeWith<S> + EncodeWith<S>,
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
