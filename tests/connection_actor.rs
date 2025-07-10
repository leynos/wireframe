//! Tests for the `ConnectionActor` component.
//!
//! These cover priority order, shutdown behaviour, error propagation,
//! interleaved cancellation and back-pressure handling.

use futures::stream;
use rstest::{fixture, rstest};
use tokio::{
    sync::oneshot,
    time::{Duration, sleep, timeout},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use wireframe::{
    connection::{ConnectionActor, FairnessConfig},
    push::PushQueues,
    response::{FrameStream, WireframeError},
};

#[fixture]
#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
fn queues() -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) { PushQueues::bounded(8, 8) }

#[fixture]
#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[fixture]
#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
fn empty_stream() -> Option<FrameStream<u8, ()>> { None }

#[rstest]
#[tokio::test]
async fn strict_priority_order(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    handle.push_low_priority(2).await.unwrap();
    handle.push_high_priority(1).await.unwrap();

    let stream = stream::iter(vec![Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(out, vec![1, 2, 3]);
}

#[rstest]
#[tokio::test]
async fn fairness_yields_low_after_burst(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    let fairness = FairnessConfig {
        max_high_before_low: 2,
        time_slice: None,
    };

    for n in 1..=5 {
        handle.push_high_priority(n).await.unwrap();
    }
    handle.push_low_priority(99).await.unwrap();

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, None, shutdown_token);
    actor.set_fairness(fairness);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(out, vec![1, 2, 99, 3, 4, 5]);
}

#[derive(Debug, Clone, Copy)]
enum Priority {
    High,
    Low,
}

/// Push frames in the given priority order and return the expected output
/// sequence when fairness is disabled.
async fn queue_frames(
    order: &[Priority],
    handle: &wireframe::push::PushHandle<u8>,
    high_count: usize,
) -> Vec<u8> {
    let mut next_high = 1u8;
    let mut next_low = u8::try_from(high_count).expect("too many high frames") + 1;

    let mut highs = Vec::new();
    let mut lows = Vec::new();

    for priority in order {
        match priority {
            Priority::High => {
                let msg = format!("failed to push high-priority frame {next_high}");
                handle.push_high_priority(next_high).await.expect(&msg);
                highs.push(next_high);
                next_high += 1;
            }
            Priority::Low => {
                let msg = format!("failed to push low-priority frame {next_low}");
                handle.push_low_priority(next_low).await.expect(&msg);
                lows.push(next_low);
                next_low += 1;
            }
        }
    }

    highs.into_iter().chain(lows.into_iter()).collect()
}

// Ensure the helper correctly handles edge cases without queued frames.
#[rstest]
#[tokio::test]
async fn queue_frames_empty_input(queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>)) {
    let (_, handle) = queues;
    let priorities: &[Priority] = &[];
    let result = queue_frames(priorities, &handle, 0).await;
    assert!(result.is_empty(), "Expected empty output for empty input");
}

#[rstest]
#[case(Vec::new())]
#[case(vec![Priority::High])]
#[case(vec![Priority::Low])]
#[case(vec![Priority::High, Priority::Low])]
#[case(vec![Priority::High; 3])]
#[case(vec![Priority::Low; 3])]
#[case(vec![Priority::High, Priority::High, Priority::High, Priority::Low, Priority::Low])]
#[case(vec![Priority::Low, Priority::Low, Priority::High, Priority::High, Priority::High])]
#[case(vec![
    Priority::High,
    Priority::Low,
    Priority::High,
    Priority::Low,
    Priority::High,
])]
#[tokio::test]
async fn processes_all_priorities_in_order(
    #[case] order: Vec<Priority>,
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    let fairness = FairnessConfig {
        max_high_before_low: 0,
        time_slice: None,
    };

    let high_count = order.iter().filter(|p| matches!(p, Priority::High)).count();
    let expected = queue_frames(&order, &handle, high_count).await;

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, None, shutdown_token);
    actor.set_fairness(fairness);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(out, expected);
}

#[rstest]
#[tokio::test]
async fn fairness_yields_low_with_time_slice(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    // Use Tokio's virtual clock so timing-dependent fairness is deterministic.
    tokio::time::pause();
    let (queues, handle) = queues;
    let fairness = FairnessConfig {
        max_high_before_low: 0,
        time_slice: Some(Duration::from_millis(10)),
    };

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle.clone(), None, shutdown_token);
    actor.set_fairness(fairness);

    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut out = Vec::new();
        let _ = actor.run(&mut out).await;
        let _ = tx.send(out);
    });

    handle.push_high_priority(1).await.unwrap();
    tokio::time::advance(Duration::from_millis(5)).await;
    handle.push_high_priority(2).await.unwrap();
    tokio::time::advance(Duration::from_millis(15)).await;
    handle.push_low_priority(42).await.unwrap();
    for n in 3..=5 {
        handle.push_high_priority(n).await.unwrap();
    }
    drop(handle);

    let out = rx.await.unwrap();
    assert!(out.contains(&42), "Low-priority item was not yielded");
    let pos = out.iter().position(|x| *x == 42).unwrap();
    assert!(
        pos > 0 && pos < out.len() - 1,
        "Low-priority item should be yielded in the middle"
    );
}

#[rstest]
#[tokio::test]
async fn shutdown_signal_precedence(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    shutdown_token.cancel();
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, None, shutdown_token);
    // drop the handle after actor creation to mimic early disconnection
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert!(out.is_empty());
}

#[rstest]
#[tokio::test]
async fn complete_draining_of_sources(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    handle.push_high_priority(1).await.unwrap();

    let stream = stream::iter(vec![Ok(2u8), Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    // drop handle after actor setup
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(out, vec![1, 2, 3]);
}

#[derive(Debug)]
enum TestError {
    Kaboom,
}

#[rstest]
#[tokio::test]
async fn error_propagation_from_stream(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    let stream = stream::iter(vec![
        Ok(1u8),
        Ok(2u8),
        Err(WireframeError::Protocol(TestError::Kaboom)),
    ]);
    let called = Arc::new(AtomicUsize::new(0));
    let c = called.clone();
    let hooks = ProtocolHooks {
        handle_error: Some(Box::new(
            move |_e: TestError, _ctx: &mut ConnectionContext| {
                c.fetch_add(1, Ordering::SeqCst);
            },
        )),
        ..ProtocolHooks::<u8, TestError>::default()
    };
    let mut actor: ConnectionActor<_, TestError> = ConnectionActor::with_hooks(
        queues,
        handle,
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert_eq!(out, vec![1, 2]);
}

#[rstest]
#[tokio::test]
#[serial]
async fn protocol_error_logs_warning(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
    mut logger: LoggerHandle,
) {
    let (queues, handle) = queues;
    let stream = stream::iter(vec![Err(WireframeError::Protocol(TestError::Kaboom))]);
    let mut actor: ConnectionActor<_, TestError> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert!(out.is_empty());
    let record = logger.pop().expect("expected warning");
    assert_eq!(record.level(), log::Level::Warn);
    assert!(record.args().contains("protocol error"));
}

#[rstest]
#[tokio::test]
async fn io_error_terminates_connection(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    let stream = stream::iter(vec![
        Ok(1u8),
        Err(WireframeError::Io(std::io::Error::other("fail"))),
    ]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    let result = actor.run(&mut out).await;
    assert!(matches!(result, Err(WireframeError::Io(_))));
    assert_eq!(out, vec![1]);
}

#[rstest]
#[tokio::test]
async fn interleaved_shutdown_during_stream(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    let token = shutdown_token.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        token.cancel();
    });

    let stream = stream::unfold(1u8, |i| async move {
        if i <= 5 {
            sleep(Duration::from_millis(20)).await;
            Some((Ok(i), i + 1))
        } else {
            None
        }
    });
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert!(!out.is_empty() && out.len() < 5);
}

#[rstest]
#[tokio::test]
async fn push_queue_exhaustion_backpressure() {
    let (mut queues, handle) = PushQueues::bounded(1, 1);
    handle.push_high_priority(1).await.unwrap();

    let blocked = timeout(Duration::from_millis(50), handle.push_high_priority(2)).await;
    assert!(blocked.is_err());

    // clean up without exposing internal fields
    queues.close();
}

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serial_test::serial;
use wireframe::{ConnectionContext, ProtocolHooks};
use wireframe_testing::{LoggerHandle, logger};

#[rstest]
#[tokio::test]
async fn before_send_hook_modifies_frames(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    handle.push_high_priority(1).await.unwrap();

    let stream = stream::iter(vec![Ok(2u8)]);
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(|f: &mut u8, _ctx: &mut ConnectionContext| *f += 1)),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        queues,
        handle,
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(out, vec![2, 3]);
}

#[rstest]
#[tokio::test]
async fn on_command_end_hook_runs(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    let stream = stream::iter(vec![Ok(1u8)]);

    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let hooks = ProtocolHooks {
        on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
            c.fetch_add(1, Ordering::SeqCst);
        })),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        queues,
        handle,
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[rstest]
#[tokio::test]
async fn graceful_shutdown_waits_for_tasks() {
    let tracker = TaskTracker::new();
    let token = CancellationToken::new();

    let mut handles = Vec::new();
    for _ in 0..5 {
        let (queues, handle) = PushQueues::<u8>::bounded(1, 1);
        let mut actor: ConnectionActor<_, ()> =
            ConnectionActor::new(queues, handle.clone(), None, token.clone());
        handles.push(handle);
        tracker.spawn(async move {
            let mut out = Vec::new();
            let _ = actor.run(&mut out).await;
        });
    }

    token.cancel();
    tracker.close();

    assert!(
        timeout(Duration::from_millis(500), tracker.wait())
            .await
            .is_ok()
    );
}
