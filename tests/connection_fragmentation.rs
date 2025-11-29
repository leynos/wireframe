//! Tests for `ConnectionActor` outbound fragmentation behaviour.
//!
//! Verifies that frames exceeding the fragment payload cap are split into
//! multiple fragments and that small frames pass through unfragmented.
#![cfg(not(loom))]

use std::{num::NonZeroUsize, time::Duration};

use tokio_util::sync::CancellationToken;
use wireframe::{
    ConnectionActor,
    app::{Envelope, Packet},
    fragment::{FragmentationConfig, Reassembler, decode_fragment_payload},
    push::{PushHandle, PushQueues},
};

const ROUTE_ID: u32 = 7;

fn setup_fragmented_actor() -> (
    ConnectionActor<Envelope, ()>,
    PushHandle<Envelope>,
    FragmentationConfig,
) {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()
        .expect("build queues");
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle.clone(), None, shutdown);

    let cfg = FragmentationConfig::for_frame_budget(
        96,
        NonZeroUsize::new(256).expect("non-zero message cap"),
        Duration::from_secs(5),
    )
    .expect("frame budget must exceed overhead");
    actor.enable_fragmentation(cfg);
    (actor, handle, cfg)
}

#[tokio::test]
async fn connection_actor_fragments_outbound_frames() {
    let (mut actor, handle, cfg) = setup_fragmented_actor();

    let cap = cfg.fragment_payload_cap.get();
    let payload = vec![1_u8; cap.saturating_add(16)];
    let frame = Envelope::new(ROUTE_ID, Some(9), payload.clone());
    handle.push_low_priority(frame).await.expect("push frame");
    drop(handle);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert!(
        out.len() > 1,
        "fragmentation should yield multiple frames, got {}",
        out.len()
    );

    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);
    let mut assembled: Option<Vec<u8>> = None;
    for env in out {
        let payload = env.into_parts().payload();
        if let Some((header, frag)) = decode_fragment_payload(&payload).expect("decode payload") {
            if let Some(message) = reassembler.push(header, frag).expect("reassemble fragment") {
                assembled = Some(message.into_payload());
            }
        } else {
            assembled = Some(payload);
        }
    }

    assert_eq!(assembled.expect("assembled payload"), payload);
}

#[tokio::test]
async fn connection_actor_passes_through_small_outbound_frames_unfragmented() {
    let (mut actor, handle, cfg) = setup_fragmented_actor();

    let payload_cap = cfg.fragment_payload_cap.get();
    let payload = vec![5_u8; payload_cap.saturating_sub(1)];
    let frame = Envelope::new(ROUTE_ID, Some(1), payload.clone());
    handle.push_low_priority(frame).await.expect("push frame");
    drop(handle);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out.len(), 1, "expected unfragmented single frame");
    let only = out.into_iter().next().expect("frame present");
    let payload_out = only.into_parts().payload();
    match decode_fragment_payload(&payload_out) {
        Ok(None) => {}
        other => panic!("expected unfragmented payload, got {other:?}"),
    }
    assert_eq!(payload_out, payload);
}
