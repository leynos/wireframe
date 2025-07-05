//! Tests for the `ConnectionActor` component.
//!
//! These cover priority order, shutdown behaviour, error propagation,
//! interleaved cancellation and back-pressure handling.

use futures::stream;
use rstest::{fixture, rstest};
use tokio::time::{Duration, sleep, timeout};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use wireframe::{
    connection::{ConnectionActor, FairnessConfig},
    push::PushQueues,
    response::{FrameStream, WireframeError},
};

#[fixture]
#[allow(unused_braces)]
fn queues() -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) { PushQueues::bounded(8, 8) }

#[fixture]
#[allow(unused_braces)]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[fixture]
#[allow(unused_braces)]
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

use wireframe::{ConnectionContext, ProtocolHooks};

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
