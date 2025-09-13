#![cfg(all(feature = "advanced-tests", not(loom)))]
//! Advanced property-based fuzzing tests for push, stream, and protocol parsing.
//!
//! This module provides comprehensive fuzzing tests using proptest to verify
//! the correctness of push queue priorities, stream frame handling, and envelope parsing in
//! various randomized scenarios.

use futures::stream;
use proptest::prelude::*;
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::Envelope,
    connection::ConnectionActor,
    response::FrameStream,
};

#[cfg(feature = "serializer-bincode")]
use wireframe::serializer::BincodeSerializer;


#[path = "../support.rs"]
mod support;

#[derive(Debug, Clone)]
enum Action {
    High(u8),
    Low(u8),
    Stream(Vec<u8>),
}

async fn run_actions(actions: &[Action]) -> Vec<u8> {
    let (queues, handle) = support::builder::<u8>()
        .high_capacity(16)
        .low_capacity(16)
        .unlimited()
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();

    let mut resp_stream: Option<FrameStream<u8, ()>> = None;
    for act in actions {
        match act {
            Action::High(f) => handle
                .push_high_priority(*f)
                .await
                .expect("failed to push high priority frame"),
            Action::Low(f) => handle
                .push_low_priority(*f)
                .await
                .expect("failed to push low priority frame"),
            Action::Stream(frames) => {
                let s = stream::iter(frames.clone().into_iter().map(Ok));
                resp_stream = Some(Box::pin(s));
            }
        }
    }

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, resp_stream, shutdown);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .expect("connection actor failed to run");
    out
}

fn expected_from(actions: &[Action]) -> Vec<u8> {
    let mut high = Vec::new();
    let mut low = Vec::new();
    let mut stream_frames = Vec::new();
    for act in actions {
        match act {
            Action::High(f) => high.push(*f),
            Action::Low(f) => low.push(*f),
            Action::Stream(v) => stream_frames = v.clone(),
        }
    }
    let mut expected = high;
    expected.extend(low);
    expected.extend(stream_frames);
    expected
}

prop_compose! {
    fn actions_strategy()
        (
            high in proptest::collection::vec(any::<u8>(), 0..5),
            low in proptest::collection::vec(any::<u8>(), 0..5),
            stream_frames in proptest::collection::vec(any::<u8>(), 0..5)
        ) -> Vec<Action> {
            let mut actions = Vec::new();
            for n in high { actions.push(Action::High(n)); }
            for n in low { actions.push(Action::Low(n)); }
            if !stream_frames.is_empty() {
                actions.push(Action::Stream(stream_frames));
            }
            actions
        }
}

proptest! {
    #[test]
    fn random_push_and_stream(actions in actions_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        rt.block_on(async {
            let out = run_actions(&actions).await;
            let expected = expected_from(&actions);
            prop_assert_eq!(out, expected);
        });
    }
}

#[rstest]
#[case::empty(Vec::new())]
#[case::maximal({
    let mut actions = Vec::new();
    for n in 0u8..5 { actions.push(Action::High(n)); }
    for n in 5u8..10 { actions.push(Action::Low(n)); }
    let stream_frames = (10u8..15).collect::<Vec<_>>();
    actions.push(Action::Stream(stream_frames));
    actions
})]
#[tokio::test]
async fn test_boundary_cases(#[case] actions: Vec<Action>) {
    let out = run_actions(&actions).await;
    let expected = expected_from(&actions);
    assert_eq!(out, expected);
}

#[cfg(feature = "serializer-bincode")]
prop_compose! {
    fn envelope_strategy()
        (id in any::<u32>(), correlation in proptest::option::of(any::<u64>()), payload in proptest::collection::vec(any::<u8>(), 0..32))
        -> Envelope {
            Envelope::new(id, correlation, payload.into())
        }
}
#[cfg(feature = "serializer-bincode")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100_000))]
    #[test]
    fn envelope_roundtrip(env in envelope_strategy(), extra in proptest::collection::vec(any::<u8>(), 0..32)) {
        let serializer = BincodeSerializer;
        let mut bytes = env.to_bytes().expect("failed to serialize envelope");
        let len = bytes.len();
        bytes.extend(extra);
        let (parsed, consumed) = serializer.parse(&bytes).expect("failed to parse envelope");
        prop_assert_eq!(parsed, env);
        prop_assert_eq!(consumed, len);
    }
}

#[cfg(feature = "serializer-bincode")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100_000))]
    #[test]
    fn fuzz_parse_does_not_panic(data in proptest::collection::vec(any::<u8>(), 0..64)) {
        let serializer = BincodeSerializer;
        let _ = serializer.parse(&data);
    }
}
