//! Unit tests for push queue construction and cloning.

use tokio::sync::mpsc;

use super::{PushConfigError, PushQueues, PushQueuesBuilder};
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

#[test]
fn validate_rejects_zero_capacity() {
    let builder = PushQueuesBuilder::<u8>::default().high_capacity(0);

    assert!(
        matches!(
            builder.validate(),
            Err(PushConfigError::InvalidCapacity { .. })
        ),
        "zero capacity must be rejected before channels are allocated"
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
fn validate_rejects_out_of_range_rate() {
    let builder = PushQueuesBuilder::<u8>::default().rate(Some(0));

    assert!(
        matches!(builder.validate(), Err(PushConfigError::InvalidRate(0))),
        "zero rate must be rejected before channels are allocated"
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
