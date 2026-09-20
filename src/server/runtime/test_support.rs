//! Shared fixtures for server runtime unit tests.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Barrier, Notify};

use crate::{
    app::Envelope,
    middleware::{HandlerService, Transform},
};

/// Middleware that holds application preparation until the test releases it.
pub(super) struct PreparationBarrier {
    /// Announces that preparation reached the blocking transform.
    pub(super) entered: Arc<Notify>,
    /// Coordinates preparation with the test's shutdown signal.
    pub(super) barrier: Arc<Barrier>,
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for PreparationBarrier {
    type Output = HandlerService<Envelope>;

    /// Wait until the test lets the preparation transform continue.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.entered.notify_one();
        self.barrier.wait().await;
        service
    }
}
