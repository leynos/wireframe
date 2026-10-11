//! Shared fixtures for server runtime unit tests.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Barrier, Notify};

use crate::{
    WireframeError,
    app::{Envelope, Handler, WireframeApp},
    middleware::{HandlerService, Transform},
};

/// Middleware that holds application preparation until the test releases it.
pub(super) struct PreparationBarrier {
    /// Announces that preparation reached the blocking transform.
    entered: Arc<Notify>,
    /// Coordinates preparation with the test's shutdown signal.
    barrier: Arc<Barrier>,
}

/// Build a factory whose application preparation waits for the test barrier.
pub(super) fn preparation_factory() -> (
    Arc<Notify>,
    Arc<Barrier>,
    impl Fn() -> Result<WireframeApp, WireframeError> + Clone,
) {
    let entered = Arc::new(Notify::new());
    let barrier = Arc::new(Barrier::new(2));
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let factory = {
        let entered = Arc::clone(&entered);
        let barrier = Arc::clone(&barrier);
        move || -> Result<WireframeApp, WireframeError> {
            WireframeApp::new()?
                .route(1, Arc::clone(&handler))?
                .wrap(PreparationBarrier {
                    entered: Arc::clone(&entered),
                    barrier: Arc::clone(&barrier),
                })
        }
    };
    (entered, barrier, factory)
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
