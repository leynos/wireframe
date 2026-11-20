//! Unit tests for push queue construction and cloning.

use tokio::sync::mpsc;

use super::{MAX_PUSH_RATE, PushConfigError, PushQueues, PushQueuesBuilder};
use crate::push::{PushError, PushPolicy, PushPriority};

#[path = "tests/rate_permits.rs"]
mod rate_permits;

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
