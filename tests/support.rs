#![cfg_attr(loom, allow(missing_docs))]
#![cfg(not(loom))]
//! Shared fixtures for integration tests.

use wireframe::push::{PushQueues, PushQueuesBuilder};

/// Returns a builder with unit capacities for reuse across tests.
#[must_use]
pub fn builder<F: Send + 'static>() -> PushQueuesBuilder<F> {
    PushQueues::<F>::builder().high_capacity(1).low_capacity(1)
}
