#![cfg(not(loom))]
//! Integration tests for multi-packet streaming responses.
//!
//! These tests exercise the `ConnectionActor` end-to-end to emulate a client
//! receiving multiple frames for a single request. They cover graceful stream
//! completion, abrupt producer disconnects, and interleaving with other
//! responses to ensure correlation identifiers allow clients to demultiplex
//! concurrent activity.

use std::sync::{Arc, Mutex};

use rstest::rstest;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet, PacketParts},
    connection::ConnectionActor,
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushHandle, PushQueues},
};
use wireframe_testing::{LoggerHandle, logger};

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
    fn new() -> Self {
        let (queues, handle) = PushQueues::<Envelope>::builder()
            .high_capacity(4)
            .low_capacity(4)
            .unlimited()
            .build()
            .expect("failed to build PushQueues");
        let shared_handle: Arc<Mutex<Option<PushHandle<Envelope>>>> = Arc::new(Mutex::new(None));
        let handle_slot = Arc::clone(&shared_handle);
        let hooks = ProtocolHooks {
            on_connection_setup: Some(Box::new(move |handle, _ctx| {
                *handle_slot.lock().expect("handle slot poisoned") = Some(handle);
            })),
            stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| {
                Some(terminator_frame())
            })),
            ..ProtocolHooks::default()
        };

        let shutdown = CancellationToken::new();
        let actor = ConnectionActor::with_hooks(queues, handle, None, shutdown, hooks);
        let handle = shared_handle
            .lock()
            .expect("handle slot poisoned")
            .take()
            .expect("connection setup hook did not run");

        Self {
            actor,
            handle: Some(handle),
        }
    }

    fn handle(&self) -> &PushHandle<Envelope> {
        self.handle.as_ref().expect("push handle already released")
    }

    fn release_handle(&mut self) { self.handle.take(); }
}

fn parts(frame: &Envelope) -> PacketParts { frame.clone().into_parts() }

#[tokio::test]
async fn client_receives_multi_packet_stream_with_terminator() {
    let mut harness = ActorHarness::new();
    let (tx, rx) = mpsc::channel(4);
    let correlation = Some(88_u64);

    for chunk in [&[1_u8][..], &[2, 3][..]] {
        tx.send(envelope_with_payload(STREAM_ID, None, chunk))
            .await
            .expect("send frame");
    }
    drop(tx);

    harness
        .actor
        .set_multi_packet_with_correlation(Some(rx), correlation);

    harness.release_handle();

    let mut out = Vec::new();
    harness
        .actor
        .run(&mut out)
        .await
        .expect("connection actor run failed");

    assert_eq!(out.len(), 3, "expected two frames plus terminator");
    let payloads: Vec<Vec<u8>> = out.iter().map(|frame| parts(frame).payload()).collect();
    assert_eq!(payloads[0], vec![1]);
    assert_eq!(payloads[1], vec![2, 3]);
    assert_eq!(
        payloads[2],
        Vec::<u8>::new(),
        "terminator payload should be empty"
    );

    for frame in &out {
        assert_eq!(
            parts(frame).correlation_id(),
            correlation,
            "correlation id mismatch",
        );
    }
}

#[rstest]
#[tokio::test]
async fn multi_packet_logs_disconnected_when_sender_dropped(mut logger: LoggerHandle) {
    logger.clear();
    let mut harness = ActorHarness::new();
    let (tx, rx) = mpsc::channel(1);
    let correlation = Some(41_u64);
    drop(tx);

    harness
        .actor
        .set_multi_packet_with_correlation(Some(rx), correlation);

    harness
        .actor
        .set_fairness(wireframe::connection::FairnessConfig {
            max_high_before_low: 1,
            time_slice: None,
        });

    harness
        .handle()
        .push_high_priority(envelope_with_payload(11, Some(5), b"hi"))
        .await
        .expect("push high priority frame");

    harness.release_handle();

    let mut out = Vec::new();
    harness
        .actor
        .run(&mut out)
        .await
        .expect("connection actor run failed");

    assert_eq!(out.len(), 2, "expected push frame followed by terminator");
    let last = out.last().expect("terminator missing");
    assert_eq!(
        parts(last).correlation_id(),
        correlation,
        "terminator correlation mismatch",
    );

    let mut saw_disconnect = false;
    while let Some(record) = logger.pop() {
        let msg = record.args().to_string();
        if msg.contains("multi-packet stream closed") && msg.contains("reason=disconnected") {
            saw_disconnect = true;
            break;
        }
    }
    assert!(saw_disconnect, "missing disconnect log");
}

#[tokio::test]
async fn interleaved_multi_packet_and_push_frames_preserve_correlations() {
    let mut harness = ActorHarness::new();
    let (tx, rx) = mpsc::channel(4);
    let stream_correlation = Some(73_u64);

    for payload in [&[10_u8][..], &[20][..], &[30][..]] {
        tx.send(envelope_with_payload(STREAM_ID, None, payload))
            .await
            .expect("send frame");
    }
    drop(tx);

    harness
        .actor
        .set_multi_packet_with_correlation(Some(rx), stream_correlation);
    harness
        .actor
        .set_fairness(wireframe::connection::FairnessConfig {
            max_high_before_low: 1,
            time_slice: None,
        });

    harness
        .handle()
        .push_high_priority(envelope_with_payload(2, Some(1), b"A"))
        .await
        .expect("push first high priority frame");
    harness
        .handle()
        .push_low_priority(envelope_with_payload(3, Some(2), b"B"))
        .await
        .expect("push first low priority frame");
    harness
        .handle()
        .push_high_priority(envelope_with_payload(4, Some(3), b"C"))
        .await
        .expect("push second high priority frame");
    harness
        .handle()
        .push_low_priority(envelope_with_payload(5, Some(4), b"D"))
        .await
        .expect("push second low priority frame");

    harness.release_handle();

    let mut out = Vec::new();
    harness
        .actor
        .run(&mut out)
        .await
        .expect("connection actor run failed");

    let correlations: Vec<Option<u64>> = out
        .iter()
        .map(|frame| parts(frame).correlation_id())
        .collect();
    assert_eq!(
        correlations,
        vec![
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            stream_correlation,
            stream_correlation,
            stream_correlation,
            stream_correlation
        ],
        "unexpected correlation ordering",
    );

    let ids: Vec<u32> = out.iter().map(|frame| parts(frame).id()).collect();
    assert_eq!(
        ids,
        vec![2, 3, 4, 5, STREAM_ID, STREAM_ID, STREAM_ID, TERMINATOR_ID],
        "frame sequence did not preserve request identities",
    );
}
