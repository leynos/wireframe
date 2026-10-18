//! Tests for `ConnectionActor` outbound fragmentation behaviour.
//!
//! Verifies that frames exceeding the fragment payload cap are split into
//! multiple fragments and that small frames pass through unfragmented.
#![cfg(not(loom))]

use std::{io, num::NonZeroUsize, time::Duration};

use rstest::{fixture, rstest};
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet},
    connection::ConnectionActor,
    fragment::{FragmentationConfig, Reassembler, decode_fragment_payload},
    push::{PushHandle, PushQueues},
};
use wireframe_testing::TestResult;

const ROUTE_ID: u32 = 7;

type FragmentedActor = (
    ConnectionActor<Envelope, wireframe::NoProtocolError>,
    PushHandle<Envelope>,
    FragmentationConfig,
);

#[fixture]
fn fragmented_actor() -> TestResult<FragmentedActor> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()?;
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<_, wireframe::NoProtocolError> =
        ConnectionActor::new(queues, handle.clone(), None, shutdown);

    let message_cap = NonZeroUsize::new(256).ok_or("message cap must be non-zero")?;
    let cfg = FragmentationConfig::for_frame_budget(96, message_cap, Duration::from_secs(5))
        .ok_or("frame budget must exceed overhead")?;
    actor.enable_fragmentation(cfg);
    Ok((actor, handle, cfg))
}

#[rstest]
#[tokio::test]
async fn connection_actor_fragments_outbound_frames(
    fragmented_actor: TestResult<FragmentedActor>,
) -> TestResult {
    let (mut actor, handle, cfg) = fragmented_actor?;

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

    if out.len() <= 1 {
        return Err(format!(
            "fragmentation should yield multiple frames, got {}",
            out.len()
        )
        .into());
    }

    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);
    let mut assembled: Option<Vec<u8>> = None;
    for env in out {
        let payload = env.into_parts().into_payload();
        let Some((header, frag)) = decode_fragment_payload(&payload)? else {
            assembled = Some(payload);
            continue;
        };

        if let Some(message) = reassembler.push(header, frag)? {
            assembled = Some(message.into_payload());
        }
    }

    let assembled = assembled.ok_or("missing reassembled payload")?;
    if assembled != payload {
        return Err(format!(
            "reassembled payload mismatch: expected {payload:?}, got {assembled:?}"
        )
        .into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn connection_actor_advances_fragment_ids_across_frames(
    fragmented_actor: TestResult<FragmentedActor>,
) -> TestResult {
    let (mut actor, handle, cfg) = fragmented_actor?;

    let payload = vec![3_u8; cfg.fragment_payload_cap.get().saturating_add(16)];
    for correlation_id in [Some(9), Some(10)] {
        handle
            .push_low_priority(Envelope::new(ROUTE_ID, correlation_id, payload.clone()))
            .await?;
    }
    drop(handle);

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|err| io::Error::other(format!("actor run failed: {err:?}")))?;

    let mut message_ids = Vec::new();
    for env in out {
        let payload = env.into_parts().into_payload();
        let Some((header, _)) = decode_fragment_payload(&payload)? else {
            return Err("expected every outbound frame to be fragmented".into());
        };
        let message_id = header.message_id().get();
        if message_ids.last() != Some(&message_id) {
            message_ids.push(message_id);
        }
    }

    if message_ids != [0, 1] {
        return Err(format!("expected sequential fragment IDs [0, 1], got {message_ids:?}").into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn connection_actor_passes_through_small_outbound_frames_unfragmented(
    fragmented_actor: TestResult<FragmentedActor>,
) -> TestResult {
    let (mut actor, handle, cfg) = fragmented_actor?;

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

    if out.len() != 1 {
        return Err(format!("expected unfragmented single frame, got {}", out.len()).into());
    }
    let only = out
        .into_iter()
        .next()
        .ok_or("expected single frame but none found")?;
    let payload_out = only.into_parts().into_payload();
    match decode_fragment_payload(&payload_out)? {
        None => {}
        Some(_) => return Err("expected unfragmented payload".into()),
    }
    if payload_out != payload {
        return Err(format!(
            "payload mutated during round trip: expected {payload:?}, got {payload_out:?}"
        )
        .into());
    }
    Ok(())
}
