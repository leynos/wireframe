//! Unit tests for push queue construction and cloning.

use tokio::sync::mpsc;

use super::{PushConfigError, PushQueues, PushQueuesBuilder};
use crate::push::{PushPolicy, PushPriority};

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

#[test]
fn validate_rejects_out_of_range_rate() {
    let builder = PushQueuesBuilder::<u8>::default().rate(Some(0));

    assert!(
        matches!(builder.validate(), Err(PushConfigError::InvalidRate(0))),
        "zero rate must be rejected before channels are allocated"
    );
}
