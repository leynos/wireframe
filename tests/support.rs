//! Shared fixtures for integration tests.

use wireframe::push::{PushQueues, PushQueuesBuilder};

/// Returns a builder with unit capacities for reuse across tests.
#[must_use]
pub fn builder() -> PushQueuesBuilder<u8> {
    PushQueues::<u8>::builder().high_capacity(1).low_capacity(1)
}
