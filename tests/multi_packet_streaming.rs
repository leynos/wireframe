#![cfg(not(loom))]
//! Integration tests for multi-packet streaming responses.
//!
//! These tests exercise the `ConnectionActor` end-to-end to emulate a client
//! receiving multiple frames for a single request. They cover graceful stream
//! completion, abrupt producer disconnects, and interleaving with other
//! responses to ensure correlation identifiers allow clients to demultiplex
//! concurrent activity.

use std::sync::{Arc, OnceLock};

use log::Level as LogLevel;
use logtest as flexi_logger;
use rstest::rstest;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet, PacketParts},
    connection::{ConnectionActor, ConnectionChannels, FairnessConfig},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushHandle, PushQueues},
};
use wireframe_testing::{LoggerHandle, logger};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const STREAM_ID: u32 = 7;
const TERMINATOR_ID: u32 = 255;

fn terminator_frame() -> Envelope { Envelope::new(TERMINATOR_ID, None, Vec::new()) }

fn envelope_with_payload(id: u32, correlation: Option<u64>, payload: &[u8]) -> Envelope {
    Envelope::new(id, correlation, payload.to_vec())
}

struct ActorHarness {
    actor: ConnectionActor<Envelope, ()>,
    handle: Option<PushHandle<Envelope>>,
}

impl ActorHarness {
    fn new() -> TestResult<Self> {
        let (queues, handle) = PushQueues::<Envelope>::builder()
            .high_capacity(4)
            .low_capacity(4)
            .unlimited()
            .build()?;
        let shared_handle: Arc<OnceLock<PushHandle<Envelope>>> = Arc::new(OnceLock::new());
        let handle_slot = Arc::clone(&shared_handle);
        let hooks = ProtocolHooks {
            on_connection_setup: Some(Box::new(move |handle, _ctx| {
                if handle_slot.set(handle).is_err() {
                    panic!("push handle already captured");
                }
            })),
            stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| {
                Some(terminator_frame())
            })),
            ..ProtocolHooks::default()
        };

        let shutdown = CancellationToken::new();
        let actor = ConnectionActor::with_hooks(
            ConnectionChannels::new(queues, handle),
            None,
            shutdown,
            hooks,
        );
        let shared_handle =
            Arc::try_unwrap(shared_handle).map_err(|_| "push handle still shared at teardown")?;
        let handle = shared_handle
            .into_inner()
            .ok_or_else(|| "connection setup hook did not run")?;

        Ok(Self {
            actor,
            handle: Some(handle),
        })
    }

    fn handle(&self) -> TestResult<&PushHandle<Envelope>> {
        self.handle
            .as_ref()
            .ok_or_else(|| "push handle already released".into())
    }

    fn release_handle(&mut self) { self.handle.take(); }

    async fn run(&mut self) -> TestResult<Vec<Envelope>> {
        let mut out = Vec::new();
        self.actor.run(&mut out).await.map_err(|e| {
            Box::new(std::io::Error::other(format!(
                "connection actor run failed: {e:?}"
            ))) as Box<dyn std::error::Error + Send + Sync>
        })?;
        Ok(out)
    }
}

fn parts(frame: &Envelope) -> PacketParts { frame.clone().into_parts() }

#[tokio::test]
async fn client_receives_multi_packet_stream_with_terminator() -> TestResult<()> {
    let mut harness = ActorHarness::new()?;
    let (tx, rx) = mpsc::channel(4);
    let correlation = Some(88_u64);

    for chunk in [&[1_u8][..], &[2, 3][..]] {
        tx.send(envelope_with_payload(STREAM_ID, None, chunk))
            .await
            .map_err(|e| format!("send frame: {e}"))?;
    }
    drop(tx);

    harness
        .actor
        .set_multi_packet_with_correlation(Some(rx), correlation);

    harness.release_handle();

    let out = harness.run().await?;

    if out.len() != 3 {
        return Err("expected two frames plus terminator".into());
    }
    let payloads: Vec<Vec<u8>> = out.iter().map(|frame| parts(frame).payload()).collect();
    if payloads.get(0) != Some(&vec![1]) {
        return Err("first payload mismatch".into());
    }
    if payloads.get(1) != Some(&vec![2, 3]) {
        return Err("second payload mismatch".into());
    }
    if payloads.get(2) != Some(&Vec::<u8>::new()) {
        return Err("terminator payload should be empty".into());
    }

    for frame in &out {
        if parts(frame).correlation_id() != correlation {
            return Err("correlation id mismatch".into());
        }
    }
    Ok(())
}

fn is_disconnect_log(record: &flexi_logger::Record) -> bool {
    record.level() == LogLevel::Warn
        && record.args().contains("multi-packet stream closed")
        && record.args().contains("reason=disconnected")
}

#[rstest]
#[tokio::test]
async fn multi_packet_logs_disconnected_when_sender_dropped(
    mut logger: LoggerHandle,
) -> TestResult<()> {
    logger.clear();
    let mut harness = ActorHarness::new()?;
    let (tx, rx) = mpsc::channel(1);
    let correlation = Some(41_u64);
    drop(tx);

    harness
        .actor
        .set_multi_packet_with_correlation(Some(rx), correlation);

    harness.actor.set_fairness(interleaving_fairness());

    harness
        .handle()?
        .push_high_priority(envelope_with_payload(11, Some(5), b"hi"))
        .await?;

    harness.release_handle();

    let out = harness.run().await?;

    if out.len() != 2 {
        return Err("expected push frame followed by terminator".into());
    }
    let last = out.last().ok_or("terminator missing")?;
    if parts(last).correlation_id() != correlation {
        return Err("terminator correlation mismatch".into());
    }

    let mut saw_disconnect = false;
    while let Some(record) = logger.pop() {
        if is_disconnect_log(&record) {
            saw_disconnect = true;
            break;
        }
    }
    if !saw_disconnect {
        return Err("missing disconnect log".into());
    }
    Ok(())
}

struct FrameSpec {
    id: u32,
    correlation: u64,
    payload: &'static [u8],
}

const HIGH_PRIORITY_FRAMES: [FrameSpec; 2] = [
    FrameSpec {
        id: 2,
        correlation: 1,
        payload: b"A",
    },
    FrameSpec {
        id: 4,
        correlation: 3,
        payload: b"C",
    },
];

const LOW_PRIORITY_FRAMES: [FrameSpec; 2] = [
    FrameSpec {
        id: 3,
        correlation: 2,
        payload: b"B",
    },
    FrameSpec {
        id: 5,
        correlation: 4,
        payload: b"D",
    },
];

enum PushPriority {
    High,
    Low,
}

fn interleaving_fairness() -> FairnessConfig {
    FairnessConfig {
        max_high_before_low: 1,
        time_slice: None,
    }
}

async fn push_sequence(
    handle: &PushHandle<Envelope>,
    priority: PushPriority,
    frames: &[FrameSpec],
) -> TestResult<()> {
    for spec in frames {
        let envelope = envelope_with_payload(spec.id, Some(spec.correlation), spec.payload);
        match priority {
            PushPriority::High => handle.push_high_priority(envelope).await?,
            PushPriority::Low => handle.push_low_priority(envelope).await?,
        };
    }
    Ok(())
}

async fn setup_stream_channel(payloads: &[&[u8]]) -> TestResult<mpsc::Receiver<Envelope>> {
    let capacity = payloads.len().max(1);
    let (tx, rx) = mpsc::channel(capacity);
    for payload in payloads {
        tx.send(envelope_with_payload(STREAM_ID, None, payload))
            .await
            .map_err(|e| format!("send frame to multi-packet stream: {e}"))?;
    }
    drop(tx);
    Ok(rx)
}

async fn push_interleaved_frames(handle: &PushHandle<Envelope>) -> TestResult<()> {
    push_sequence(handle, PushPriority::High, &HIGH_PRIORITY_FRAMES).await?;
    push_sequence(handle, PushPriority::Low, &LOW_PRIORITY_FRAMES).await?;
    Ok(())
}

fn assert_correlation_ordering(frames: &[Envelope], expected: &[Option<u64>]) {
    let correlations: Vec<Option<u64>> = frames
        .iter()
        .map(|frame| parts(frame).correlation_id())
        .collect();
    assert_eq!(correlations, expected, "unexpected correlation ordering");
}

fn assert_frame_identities(frames: &[Envelope], expected: &[u32]) {
    let ids: Vec<u32> = frames.iter().map(|frame| parts(frame).id()).collect();
    assert_eq!(
        ids, expected,
        "frame sequence did not preserve request identities",
    );
}

#[tokio::test]
async fn interleaved_multi_packet_and_push_frames_preserve_correlations() -> TestResult<()> {
    let mut harness = ActorHarness::new()?;
    let stream_correlation = Some(73_u64);
    let rx = setup_stream_channel(&[&[10_u8][..], &[20][..], &[30][..]]).await?;

    harness
        .actor
        .set_multi_packet_with_correlation(Some(rx), stream_correlation);
    harness.actor.set_fairness(interleaving_fairness());

    push_interleaved_frames(harness.handle()?).await?;
    harness.release_handle();

    let frames = harness.run().await?;

    assert_correlation_ordering(
        &frames,
        &[
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            stream_correlation,
            stream_correlation,
            stream_correlation,
            stream_correlation,
        ],
    );

    assert_frame_identities(
        &frames,
        &[2, 3, 4, 5, STREAM_ID, STREAM_ID, STREAM_ID, TERMINATOR_ID],
    );
    Ok(())
}
