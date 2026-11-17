//! Rate-budget behaviour shared by the awaited and non-blocking push paths.
//!
//! One limiter serves both paths, so these cases pin that neither can overdraw
//! it: a refusal for capacity must not spend a permit, a closed connection must
//! outrank an exhausted window, and a producer parked on a full queue must not
//! bank a permit and burst once the queue drains.

use std::time::Duration;

use tokio::time;

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

    let outcome = time::timeout(Duration::from_secs(5), handle.push_high_priority(2))
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
