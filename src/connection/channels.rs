//! Connection channel bundling for actor construction.

use crate::push::{PushHandle, PushQueues};

/// Bundles push queues with their shared handle for actor construction.
pub struct ConnectionChannels<F> {
    /// Receivers for high- and low-priority frames consumed by the actor.
    pub queues: PushQueues<F>,
    /// Handle cloned by producers to enqueue frames into the shared queues.
    pub handle: PushHandle<F>,
}

impl<F> ConnectionChannels<F> {
    /// Create a new bundle of push queues and their associated handle.
    #[must_use]
    pub fn new(queues: PushQueues<F>, handle: PushHandle<F>) -> Self { Self { queues, handle } }
}
