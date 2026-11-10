//! Unit tests for push queue construction and cloning.

use super::{PushConfigError, PushQueues, PushQueuesBuilder};

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

#[test]
fn builder_clone_preserves_configuration() {
    let builder = PushQueuesBuilder::<u8>::default()
        .high_capacity(3)
        .low_capacity(2)
        .rate(Some(5))
        .dlq_log_every_n(4);

    let (_queues, _handle) = builder.clone().build().expect("cloned builder must build");
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
