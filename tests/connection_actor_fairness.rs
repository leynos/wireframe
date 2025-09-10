#![cfg(not(loom))]
//! Fairness and priority tests for `ConnectionActor`.

use futures::stream;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::{
    sync::oneshot,
    time::{self, Duration},
};
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, FairnessConfig},
    push::PushQueues,
};
use wireframe_testing::push_expect;

#[fixture]
#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
// allow(unfulfilled_lint_expectations): rustc occasionally fails to emit the expected
// lint for single-line rstest fixtures on stable.
#[allow(unfulfilled_lint_expectations)]
fn queues() -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) {
    PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .build()
        .expect("failed to build PushQueues")
}

#[fixture]
#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
// allow(unfulfilled_lint_expectations): rustc occasionally fails to emit the expected
// lint for single-line rstest fixtures on stable.
#[allow(unfulfilled_lint_expectations)]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[rstest]
#[tokio::test]
#[serial]
async fn strict_priority_order(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    push_expect!(handle.push_low_priority(2), "push low-priority");
    push_expect!(handle.push_high_priority(1), "push high-priority");

    let stream = stream::iter(vec![Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(out, vec![1, 2, 3]);
}

#[rstest]
#[tokio::test]
#[serial]
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
        push_expect!(handle.push_high_priority(n), "push high-priority");
    }
    push_expect!(handle.push_low_priority(99), "push low-priority");

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, None, shutdown_token);
    actor.set_fairness(fairness);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
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
                push_expect!(
                    handle.push_high_priority(next_high),
                    format!("push high-priority frame {next_high}")
                );
                highs.push(next_high);
                next_high += 1;
            }
            Priority::Low => {
                push_expect!(
                    handle.push_low_priority(next_low),
                    format!("push low-priority frame {next_low}")
                );
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
#[serial]
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
#[serial]
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
#[serial]
async fn fairness_yields_low_with_time_slice(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    // Use Tokio's virtual clock so timing-dependent fairness is deterministic.
    time::pause();
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

    push_expect!(handle.push_high_priority(1), "push high-priority");
    time::advance(Duration::from_millis(5)).await;
    push_expect!(handle.push_high_priority(2), "push high-priority");
    time::advance(Duration::from_millis(15)).await;
    push_expect!(handle.push_low_priority(42), "push low-priority");
    for n in 3..=5 {
        push_expect!(handle.push_high_priority(n), "push high-priority");
    }
    drop(handle);

    let out = rx.await.expect("actor output missing");
    assert!(out.contains(&42), "Low-priority item was not yielded");
    let pos = out
        .iter()
        .position(|x| *x == 42)
        .expect("value 42 should be present");
    assert!(
        pos > 0 && pos < out.len() - 1,
        "Low-priority item should be yielded in the middle",
    );
}
