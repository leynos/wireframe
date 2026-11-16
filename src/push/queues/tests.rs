//! Unit tests for push queue construction and cloning.

use std::time::Duration;

use tokio::{sync::mpsc, time};

use super::{MAX_PUSH_RATE, PushConfigError, PushQueues, PushQueuesBuilder};
use crate::push::{PushError, PushPolicy, PushPriority};

/// Frame type that deliberately does not implement `Clone`.
struct NotClone;

#[test]
fn handle_clone_does_not_require_clone_frame() {
    let (queues, handle) = PushQueues::<NotClone>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build PushQueues");

    let cloned = handle.clone();
    drop((queues, handle, cloned));
}

/// The server clones one stored template per connection, so a clone must keep
/// every setting: a lost capacity would change which pushes are dropped, and a
/// lost dead-letter sender would discard the evidence.
#[test]
fn builder_clone_preserves_configuration() {
    let (dlq_tx, mut dlq_rx) = mpsc::channel(2);
    let builder = PushQueuesBuilder::<u8>::default()
        .high_capacity(2)
        .low_capacity(1)
        .unlimited()
        .dlq(Some(dlq_tx));

    let (_queues, handle) = builder.clone().build().expect("cloned builder must build");
    for frame in [1u8, 2] {
        handle
            .try_push(frame, PushPriority::High, PushPolicy::DropIfFull)
            .expect("a two-frame capacity must accept both pushes");
    }
    handle
        .try_push(3, PushPriority::High, PushPolicy::DropIfFull)
        .expect("a full queue must dead-letter rather than fail");

    assert_eq!(
        dlq_rx.try_recv(),
        Ok(3),
        "a clone must keep the capacity and dead-letter sink it was built with"
    );
}

/// Zero is never a usable capacity or rate, so both are refused up front
/// rather than left to fail when the channels are allocated.
#[test]
fn validate_rejects_zero_capacity_and_rate() {
    let zero_capacity = PushQueuesBuilder::<u8>::default().high_capacity(0);
    assert!(
        matches!(
            zero_capacity.validate(),
            Err(PushConfigError::InvalidCapacity { .. })
        ),
        "zero capacity must be rejected before channels are allocated"
    );

    let zero_rate = PushQueuesBuilder::<u8>::default().rate(Some(0));
    assert!(
        matches!(zero_rate.validate(), Err(PushConfigError::InvalidRate(0))),
        "zero rate must be rejected before channels are allocated"
    );
}

/// A Tokio channel is backed by a semaphore, so a capacity above its limit
/// panics when the channel is allocated. Configuration must report it instead.
#[test]
fn validate_rejects_capacity_above_the_channel_limit() {
    let high_builder = PushQueuesBuilder::<u8>::default().high_capacity(usize::MAX);

    assert!(
        matches!(
            high_builder.validate(),
            Err(PushConfigError::InvalidCapacity { high, .. }) if high == usize::MAX
        ),
        "a capacity above `Semaphore::MAX_PERMITS` must be rejected"
    );

    let low_builder = PushQueuesBuilder::<u8>::default().low_capacity(usize::MAX);
    assert!(
        matches!(
            low_builder.build(),
            Err(PushConfigError::InvalidCapacity { low, .. }) if low == usize::MAX
        ),
        "building must report the bound rather than panic in `mpsc::channel`"
    );
}

#[test]
fn validate_rejects_rate_above_the_maximum() {
    let excessive = MAX_PUSH_RATE + 1;
    let builder = PushQueuesBuilder::<u8>::default().rate(Some(excessive));

    assert!(
        matches!(
            builder.validate(),
            Err(PushConfigError::InvalidRate(rate)) if rate == excessive
        ),
        "a rate above `MAX_PUSH_RATE` must be rejected"
    );
}

/// A frame the connection codec cannot encode is refused before it is queued.
///
/// A deferred push reaches the encoder unchanged, so accepting it here would
/// report success and then fail the connection when the encoder rejected the
/// frame. Both the awaited and the non-blocking push paths must report it to
/// the producer instead.
#[tokio::test]
async fn oversized_push_is_rejected_before_queueing() {
    let (mut queues, handle) = PushQueues::<Vec<u8>>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .max_frame_len(4, |frame: &Vec<u8>| frame.len())
        .build()
        .expect("queue builds");

    assert_eq!(
        handle.push_high_priority(vec![0; 5]).await,
        Err(PushError::FrameTooLarge { size: 5, max: 4 }),
        "an oversized awaited push must be reported"
    );
    assert_eq!(
        handle.try_push(vec![0; 5], PushPriority::High, PushPolicy::DropIfFull),
        Err(PushError::FrameTooLarge { size: 5, max: 4 }),
        "an oversized push must not be dropped silently"
    );

    handle
        .push_high_priority(vec![0; 4])
        .await
        .expect("a frame at the limit fits");
    let (_, frame) = queues.recv().await.expect("the accepted push is queued");
    assert_eq!(frame.len(), 4, "only the frame within the limit is queued");
}

/// A codec limit is checked as well as the template's, not instead of it.
///
/// The two limits can measure a frame differently, so the codec's measure
/// must not be applied to the template's ceiling. Here the codec would accept
/// every frame, while the template rejects anything above four bytes; the
/// frame must stay rejected.
#[tokio::test]
async fn codec_limit_does_not_relax_the_template_limit() {
    let (mut queues, handle) = PushQueues::<Vec<u8>>::builder()
        .high_capacity(2)
        .low_capacity(2)
        .unlimited()
        .max_frame_len(4, |frame: &Vec<u8>| frame.len())
        .capped_by_codec(64, |_frame: &Vec<u8>| 1)
        .build()
        .expect("queue builds");

    assert_eq!(
        handle.try_push(vec![0; 5], PushPriority::High, PushPolicy::DropIfFull),
        Err(PushError::FrameTooLarge { size: 5, max: 4 }),
        "the template's own measure must still reject the frame"
    );

    handle
        .try_push(vec![0; 4], PushPriority::High, PushPolicy::DropIfFull)
        .expect("a frame within both limits is accepted");
    let (_, frame) = queues.recv().await.expect("the accepted push is queued");
    assert_eq!(
        frame.len(),
        4,
        "only the frame within both limits is queued"
    );
}

/// A codec limit rejects a frame the template would have accepted.
///
/// Merging the two limits under the template's measure would let this frame
/// through even though the codec cannot encode it, which would end the
/// connection after the producer was told the push succeeded.
#[tokio::test]
async fn codec_limit_rejects_a_frame_the_template_allows() {
    let (_queues, handle) = PushQueues::<Vec<u8>>::builder()
        .high_capacity(2)
        .low_capacity(2)
        .unlimited()
        .max_frame_len(10, |frame: &Vec<u8>| frame.len())
        .capped_by_codec(3, |frame: &Vec<u8>| frame.len())
        .build()
        .expect("queue builds");

    assert_eq!(
        handle.try_push(vec![0; 4], PushPriority::High, PushPolicy::DropIfFull),
        Err(PushError::FrameTooLarge { size: 4, max: 3 }),
        "the codec's own ceiling must still reject the frame"
    );
}

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
