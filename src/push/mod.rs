//! Push subsystem.
//!
//! Provides queues and handles for asynchronous frame delivery.

pub mod queues;

pub use queues::{
    FrameLike,
    MAX_PUSH_RATE,
    MAX_QUEUE_CAPACITY,
    PushConfigError,
    PushError,
    PushHandle,
    PushPolicy,
    PushPriority,
    PushQueues,
    PushQueuesBuilder,
};
