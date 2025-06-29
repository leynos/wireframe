#![cfg(feature = "advanced-tests")]

use futures::stream;
use proptest::prelude::*;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::ConnectionActor,
    push::PushQueues,
    response::FrameStream,
};

#[derive(Debug, Clone)]
enum Action {
    High(u8),
    Low(u8),
    Stream(Vec<u8>),
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
            .unwrap();

        rt.block_on(async {
            let (queues, handle) = PushQueues::bounded(16, 16);
            let shutdown = CancellationToken::new();

            let mut stream: Option<FrameStream<u8, ()>> = None;
            for act in &actions {
                match act {
                    Action::High(f) => handle.push_high_priority(*f).await.unwrap(),
                    Action::Low(f) => handle.push_low_priority(*f).await.unwrap(),
                    Action::Stream(frames) => {
                        let s = stream::iter(frames.clone().into_iter().map(Ok));
                        stream = Some(Box::pin(s));
                    }
                }
            }

            let mut actor: ConnectionActor<_, ()> =
                ConnectionActor::new(queues, handle, stream, shutdown);
            let mut out = Vec::new();
            actor.run(&mut out).await.unwrap();

            let mut expected_high = Vec::new();
            let mut expected_low = Vec::new();
            let mut expected_stream = Vec::new();
            for act in actions {
                match act {
                    Action::High(f) => expected_high.push(f),
                    Action::Low(f) => expected_low.push(f),
                    Action::Stream(v) => expected_stream = v,
                }
            }
            let mut expected = expected_high;
            expected.extend(expected_low);
            expected.extend(expected_stream);

            prop_assert_eq!(out, expected);
        });
    }
}

