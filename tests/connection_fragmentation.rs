#![cfg(not(loom))]

use std::{num::NonZeroUsize, time::Duration};

use tokio_util::sync::CancellationToken;
use wireframe::{
    ConnectionActor,
    app::{Envelope, Packet},
    fragment::{FragmentationConfig, Reassembler, decode_fragment_payload},
    push::PushQueues,
};

#[tokio::test]
async fn connection_actor_fragments_outbound_frames() {
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

    let payload = vec![1_u8; 80];
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

const ROUTE_ID: u32 = 7;
