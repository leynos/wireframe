//! Prioritised queues used for asynchronously pushing frames to a connection.
//!
//! # Overview
//! Expose prioritised push queues and their handle. Construct via a fluent
//! builder.
//!
//! # Example
//! ```no_run
//! use wireframe::push::PushQueues;
//!
//! let (queues, handle) = PushQueues::builder()
//!     .high_capacity(8)
//!     .low_capacity(8)
//!     .build()
//!     .expect("failed to build PushQueues");
//! # drop((queues, handle));
//! ```

mod queues;

pub(crate) use self::queues::{FrameLike, PushHandleInner};
pub use self::queues::{
    PushConfigError,
    PushError,
    PushHandle,
    PushPolicy,
    PushPriority,
    PushQueues,
    PushQueuesBuilder,
};
