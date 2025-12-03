//! Fairness and priority tests for `ConnectionActor`.
#![cfg(not(loom))]
#![allow(
    unfulfilled_lint_expectations,
    reason = "Needed for rustc suppressing false positives"
)]

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

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[fixture]
fn queues() -> TestResult<(PushQueues<u8>, wireframe::push::PushHandle<u8>)> {
    PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .build()
        .map_err(Into::into)
}

#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[rstest]
#[tokio::test]
#[serial]
async fn strict_priority_order(
    queues: TestResult<(PushQueues<u8>, wireframe::push::PushHandle<u8>)>,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
    push_expect!(handle.push_low_priority(2), "push low-priority");
    push_expect!(handle.push_high_priority(1), "push high-priority");

    let stream = stream::iter(vec![Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("actor run failed: {e:?}")))?;
    if out != vec![1, 2, 3] {
        return Err("unexpected frame ordering".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
#[serial]
async fn fairness_yields_low_after_burst(
    queues: TestResult<(PushQueues<u8>, wireframe::push::PushHandle<u8>)>,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
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
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("actor run failed: {e:?}")))?;
    if out != vec![1, 2, 99, 3, 4, 5] {
        return Err("unexpected frame order under fairness".into());
    }
    Ok(())
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
) -> TestResult<Vec<u8>> {
    let mut next_high = 1u8;
    let mut next_low = u8::try_from(high_count).map_err(|_| "high_count exceeds u8 range")? + 1;

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

    Ok(highs.into_iter().chain(lows.into_iter()).collect())
}

// Ensure the helper correctly handles edge cases without queued frames.
#[rstest]
#[tokio::test]
#[serial]
async fn queue_frames_empty_input(
    queues: TestResult<(PushQueues<u8>, wireframe::push::PushHandle<u8>)>,
) -> TestResult {
    let (_, handle) = queues?;
    let priorities: &[Priority] = &[];
    let result = queue_frames(priorities, &handle, 0).await?;
    if !result.is_empty() {
        return Err("expected empty output for empty input".into());
    }
    Ok(())
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
    queues: TestResult<(PushQueues<u8>, wireframe::push::PushHandle<u8>)>,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
    let fairness = FairnessConfig {
        max_high_before_low: 0,
        time_slice: None,
    };

    let high_count = order.iter().filter(|p| matches!(p, Priority::High)).count();
    let expected = queue_frames(&order, &handle, high_count).await?;

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, None, shutdown_token);
    actor.set_fairness(fairness);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("actor run failed: {e:?}")))?;
    if out != expected {
        return Err("unexpected frame ordering with fairness disabled".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
#[serial]
async fn fairness_yields_low_with_time_slice(
    queues: TestResult<(PushQueues<u8>, wireframe::push::PushHandle<u8>)>,
    shutdown_token: CancellationToken,
) -> TestResult {
    // Use Tokio's virtual clock so timing-dependent fairness is deterministic.
    time::pause();
    let (queues, handle) = queues?;
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

    let out = rx.await.map_err(|_| "actor output missing")?;
    if !out.contains(&42) {
        return Err("low-priority item was not yielded".into());
    }
    let pos = out
        .iter()
        .position(|x| *x == 42)
        .ok_or("value 42 should be present")?;
    if !(pos > 0 && pos < out.len() - 1) {
        return Err("low-priority item should be yielded in the middle".into());
    }
    Ok(())
}
