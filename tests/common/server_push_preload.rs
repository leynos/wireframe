//! Preload constructor for the server push harness.
//!
//! Only the binary that asserts on frames queued during connection setup needs
//! this constructor, so it lives apart from the shared harness and is compiled
//! into that binary alone. Keeping it out of
//! [`server_push_support`](super::support) is what lets every other helper
//! there stay under normal dead-code checking in the binaries that do not
//! drive it.

use std::sync::Arc;

use bytes::Bytes;
use wireframe::push::PushPriority;

use super::support::PushTestProtocol;

impl PushTestProtocol {
    /// Build a protocol that queues `preloaded` during every setup.
    pub fn with_preloaded(preloaded: Vec<(Bytes, PushPriority)>) -> Self {
        Self {
            preloaded: Arc::new(preloaded),
            ..Self::default()
        }
    }
}