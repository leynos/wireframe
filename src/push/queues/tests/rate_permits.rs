//! Rate-budget behaviour shared by the awaited and non-blocking push paths.
//!
//! One limiter serves both paths, so these cases pin that neither can overdraw
//! it: a refusal for capacity must not spend a permit, a closed connection must
//! outrank an exhausted window, and a producer parked on a full queue must not
//! bank a permit and burst once the queue drains.

use std::time::Duration;

use proptest::{prelude::*, test_runner::TestCaseError};
use tokio::{
    runtime::{Builder, Runtime},
    time,
};

use super::super::PushQueues;
use crate::push::{PushError, PushPolicy, PushPriority};

/// `try_push` draws on the same rate budget as the awaited helpers.
///
/// The limiter allows one push per second. After an awaited push consumes that
/// permit, the non-blocking path must refuse rather than overdraw the budget,
/// and it must do so without waiting. Once the window refills, the same handle
/// clone accepts the frame.
#[tokio::test]
async fn try_push_shares_the_rate_limit_with_awaited_pushes() {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(1))
        .build()
        .expect("queue builds");
    let clone = handle.clone();

    handle
        .push_high_priority(1)
        .await
        .expect("the first push consumes the only permit");
    assert_eq!(
        clone.try_push(2, PushPriority::High, PushPolicy::DropIfFull),
        Err(PushError::RateLimited),
        "a clone must see the permit the awaited path consumed"
    );

    time::advance(Duration::from_secs(1)).await;
    clone
        .try_push(3, PushPriority::High, PushPolicy::DropIfFull)
        .expect("a replenished permit admits the non-blocking push");

    let (_, first) = queues.recv().await.expect("the awaited push is queued");
    let (_, second) = queues.recv().await.expect("the admitted push is queued");
    assert_eq!(
        (first, second),
        (1, 3),
        "the refused frame must not be queued"
    );
}

/// A non-blocking push refused for capacity spends no rate budget.
///
/// Capacity is reserved before a permit is drawn, so the refused high-priority
/// push leaves the last permit untouched. The low-priority lane has room of its
/// own, so its push must still be admitted. Drawing the permit first would have
/// spent it on the refused frame and rejected the low-priority push as
/// `RateLimited` with nothing queued.
#[tokio::test]
async fn try_push_refused_for_capacity_does_not_spend_a_rate_permit() {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .rate(Some(2))
        .build()
        .expect("queue builds");

    handle
        .push_high_priority(1)
        .await
        .expect("the first push spends one permit and fills the high lane");
    assert_eq!(
        handle.try_push(2, PushPriority::High, PushPolicy::ReturnErrorIfFull),
        Err(PushError::QueueFull),
        "a full high lane must refuse the frame without spending the last permit"
    );
    handle
        .try_push(3, PushPriority::Low, PushPolicy::ReturnErrorIfFull)
        .expect("the low lane has room and the refusal spent no permit");

    let delivered = [queues.recv().await, queues.recv().await];
    assert_eq!(
        delivered,
        [Some((PushPriority::High, 1)), Some((PushPriority::Low, 3)),],
        "only the admitted frames are queued, in delivery order"
    );
}

/// A closed connection outranks an exhausted rate limit on the non-blocking
/// path.
///
/// `RateLimited` invites the producer to retry after the next window, which is
/// advice that can never succeed once the receiver is gone. Reporting the
/// closure first ends the producer's work instead of postponing it by a refill
/// interval.
#[tokio::test]
async fn try_push_reports_a_closed_connection_before_an_exhausted_rate_limit() {
    time::pause();
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .rate(Some(1))
        .build()
        .expect("queue builds");
    handle
        .push_high_priority(1)
        .await
        .expect("the first push consumes the only permit");
    drop(queues);

    assert_eq!(
        handle.try_push(2, PushPriority::High, PushPolicy::ReturnErrorIfFull),
        Err(PushError::Closed),
        "a closed connection is permanent, so it must outrank the rate limit"
    );
}

/// An awaited push reports the closure rather than waiting for a refill.
///
/// The permit loop sleeps between attempts, so a push that only learns about
/// the closure after acquiring a token would keep the producer parked for a
/// whole refill window — and would never succeed even then.
#[tokio::test]
async fn awaited_push_reports_closed_without_waiting_for_a_rate_token() {
    time::pause();
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .rate(Some(1))
        .build()
        .expect("queue builds");
    handle
        .push_high_priority(1)
        .await
        .expect("the first push consumes the only permit");
    drop(queues);

    // Paused time auto-advances whenever the runtime goes idle, so a multi-
    // second budget would let the permit sleep elapse and hide the wait this
    // test is about. A zero budget admits only a future that is ready on its
    // first poll.
    let outcome = time::timeout(Duration::ZERO, handle.push_high_priority(2))
        .await
        .expect("a closed connection must be reported without waiting for a refill");
    assert_eq!(outcome, Err(PushError::Closed));
}

/// Producers parked on a full queue hold no rate permit while they wait.
///
/// Capacity is reserved before a permit is drawn. A producer that instead took
/// a permit first and then parked on a full queue would bank that permit, and
/// every banked frame would leave the queue together the instant the consumer
/// drained it — a burst that exceeds the configured rate.
///
/// The queue holds one frame and admits one per second, so the three parked
/// producers must leave one per second rather than all at once.
#[tokio::test]
async fn producers_parked_on_a_full_queue_do_not_burst_after_it_drains() {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .rate(Some(1))
        .build()
        .expect("queue builds");
    handle
        .push_high_priority(1)
        .await
        .expect("the first push consumes the window's permit and fills the queue");

    let parked = tokio::spawn({
        let producer = handle.clone();
        async move {
            futures::join!(
                producer.push_high_priority(2),
                producer.push_high_priority(3),
                producer.push_high_priority(4),
            )
        }
    });
    // Let the parked producers observe the full queue before time moves on.
    tokio::task::yield_now().await;
    // Time passes with the queue still full, so a producer that banked a
    // permit would have one in hand by now.
    time::advance(Duration::from_secs(3)).await;

    assert_eq!(
        queues.recv().await,
        Some((PushPriority::High, 1)),
        "the queued frame is drained first"
    );
    assert_eq!(
        queues.recv().await,
        Some((PushPriority::High, 2)),
        "the permit accumulated while parked admits the next frame"
    );
    assert!(
        time::timeout(Duration::ZERO, queues.recv()).await.is_err(),
        "the third frame must wait for the next refill, not leave with the second"
    );
    time::advance(Duration::from_secs(1)).await;
    assert_eq!(queues.recv().await, Some((PushPriority::High, 3)));
    assert!(
        time::timeout(Duration::ZERO, queues.recv()).await.is_err(),
        "the fourth frame must also wait for a refill"
    );
    time::advance(Duration::from_secs(1)).await;
    assert_eq!(queues.recv().await, Some((PushPriority::High, 4)));

    assert_eq!(
        parked.await.expect("the producer task must not panic"),
        (Ok(()), Ok(()), Ok(())),
        "every parked frame is eventually admitted"
    );
}

/// Build a current-thread runtime whose clock only moves when asked.
///
/// The limiter refills once a second, so a runtime that advanced time on its
/// own could admit a push from the next window and hide an overdraw.
fn paused_runtime() -> Result<Runtime, TestCaseError> {
    Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .map_err(|error| TestCaseError::fail(format!("a paused runtime must build: {error}")))
}

/// Build queues that admit `budget` pushes per window with room for every
/// generated attempt in either lane.
fn ticketed_queues(
    budget: u8,
) -> Result<(PushQueues<u8>, crate::push::PushHandle<u8>), TestCaseError> {
    PushQueues::<u8>::builder()
        .high_capacity(64)
        .low_capacity(64)
        .rate(Some(usize::from(budget)))
        .build()
        .map_err(|error| TestCaseError::fail(format!("the queues must build: {error}")))
}

// However the attempts are split across lanes and cloned handles, one window
// admits exactly its budget and never more.
proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn a_window_never_admits_more_pushes_than_its_budget(
        budget in 1u8..=8,
        lanes in proptest::collection::vec(any::<bool>(), 1..=32),
    ) {
        let runtime = paused_runtime()?;
        let (attempts, admitted) = runtime.block_on(async move {
            let (_queues, handle) = ticketed_queues(budget)?;
            let clone = handle.clone();
            let attempts = lanes.len();
            let mut admitted = 0usize;
            for is_high in lanes {
                let priority = if is_high {
                    PushPriority::High
                } else {
                    PushPriority::Low
                };
                if clone
                    .try_push(1, priority, PushPolicy::ReturnErrorIfFull)
                    .is_ok()
                {
                    admitted += 1;
                }
            }
            Ok::<_, TestCaseError>((attempts, admitted))
        })?;

        let budget = usize::from(budget);
        prop_assert!(
            admitted <= budget,
            "{admitted} pushes were admitted in one window with a budget of {budget}"
        );
        prop_assert_eq!(
            admitted,
            attempts.min(budget),
            "every attempt a fresh window can admit must be admitted"
        );
    }
}

// A closed connection is reported before an exhausted window, for either lane
// and any budget: `RateLimited` invites a retry that can never succeed once the
// receiver is gone.
proptest! {
    #[test]
    fn a_closed_connection_outranks_an_exhausted_window(
        budget in 1u8..=8,
        is_high in any::<bool>(),
    ) {
        let runtime = paused_runtime()?;
        let error = runtime.block_on(async move {
            let (queues, handle) = ticketed_queues(budget)?;
            for _ in 0..budget {
                handle
                    .try_push(1, PushPriority::High, PushPolicy::ReturnErrorIfFull)
                    .map_err(|error| TestCaseError::fail(format!("the budget must admit the frame: {error}")))?;
            }
            drop(queues);
            let priority = if is_high {
                PushPriority::High
            } else {
                PushPriority::Low
            };
            Ok::<_, TestCaseError>(handle.try_push(2, priority, PushPolicy::ReturnErrorIfFull))
        })?;

        prop_assert_eq!(error, Err(PushError::Closed));
    }
}
