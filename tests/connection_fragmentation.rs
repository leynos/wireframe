//! Tests for `ConnectionActor` outbound fragmentation behaviour.
//!
//! Verifies that frames exceeding the fragment payload cap are split into
//! multiple fragments and that small frames pass through unfragmented.
#![cfg(not(loom))]

use std::{io, num::NonZeroUsize, time::Duration};

use tokio_util::sync::CancellationToken;
use wireframe::{
    ConnectionActor,
    app::{Envelope, Packet},
    fragment::{FragmentationConfig, Reassembler, decode_fragment_payload},
    push::{PushHandle, PushQueues},
};

const ROUTE_ID: u32 = 7;

mod common;
use common::TestResult;

fn setup_fragmented_actor() -> TestResult<(
    ConnectionActor<Envelope, ()>,
    PushHandle<Envelope>,
    FragmentationConfig,
)> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()?;
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle.clone(), None, shutdown);

    let message_cap = NonZeroUsize::new(256).ok_or("message cap must be non-zero")?;
    let cfg = FragmentationConfig::for_frame_budget(96, message_cap, Duration::from_secs(5))
        .ok_or("frame budget must exceed overhead")?;
    actor.enable_fragmentation(cfg);
    Ok((actor, handle, cfg))
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn connection_actor_fragments_outbound_frames() -> TestResult {
    let (mut actor, handle, cfg) = setup_fragmented_actor()?;

    let cap = cfg.fragment_payload_cap.get();
    let payload = vec![1_u8; cap.saturating_add(16)];
    let frame = Envelope::new(ROUTE_ID, Some(9), payload.clone());
    handle.push_low_priority(frame).await?;
    drop(handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|err| io::Error::other(format!("actor run failed: {err:?}")))?;

    assert!(
        out.len() > 1,
        "fragmentation should yield multiple frames, got {}",
        out.len()
    );

    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);
    let mut assembled: Option<Vec<u8>> = None;
    for env in out {
        let payload = env.into_parts().payload();
        let Some((header, frag)) = decode_fragment_payload(&payload)? else {
            assembled = Some(payload);
            continue;
        };

        if let Some(message) = reassembler.push(header, frag)? {
            assembled = Some(message.into_payload());
        }
    }

    let assembled = assembled.ok_or("missing reassembled payload")?;
    assert_eq!(assembled, payload, "reassembled payload mismatch");
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn connection_actor_passes_through_small_outbound_frames_unfragmented() -> TestResult {
    let (mut actor, handle, cfg) = setup_fragmented_actor()?;

    let payload_cap = cfg.fragment_payload_cap.get();
    let payload = vec![5_u8; payload_cap.saturating_sub(1)];
    let frame = Envelope::new(ROUTE_ID, Some(1), payload.clone());
    handle.push_low_priority(frame).await?;
    drop(handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|err| io::Error::other(format!("actor run failed: {err:?}")))?;

    assert_eq!(out.len(), 1, "expected unfragmented single frame");
    let only = out
        .into_iter()
        .next()
        .ok_or("expected single frame but none found")?;
    let payload_out = only.into_parts().payload();
    match decode_fragment_payload(&payload_out)? {
        None => {}
        Some(_) => return Err("expected unfragmented payload".into()),
    }
    assert_eq!(payload_out, payload, "payload mutated during round trip");
    Ok(())
}
