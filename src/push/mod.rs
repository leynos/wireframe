//! Push subsystem.
//!
//! Provides queues and handles for asynchronous frame delivery.

pub mod queues;

pub use queues::{
    FrameLike,
    MAX_PUSH_RATE,
    PushConfigError,
    PushError,
    PushHandle,
    PushPolicy,
    PushPriority,
    PushQueues,
    PushQueuesBuilder,
};
