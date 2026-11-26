//! Pair-level cancellation coverage over the shared startup core.

use std::{future::Future, pin::Pin, sync::Arc, task::Poll, time::Duration};

use futures::poll;
use tokio::sync::{Barrier, Notify};
use wireframe::{
    app::{Envelope, Handler},
    middleware::{HandlerService, Transform},
};

use super::start_pair;
use crate::{TestResult, integration_helpers::unused_listener, wait_for_listener_release};

/// Middleware that holds application preparation until released.
struct PreparationBarrier {
    /// Fires when preparation reaches this transform.
    entered: Arc<Notify>,
    /// The test and the transform meet here.
    barrier: Arc<Barrier>,
}

impl Transform<HandlerService<Envelope>> for PreparationBarrier {
    type Output = HandlerService<Envelope>;

    fn transform<'life0, 'async_trait>(
        &'life0 self,
        service: HandlerService<Envelope>,
    ) -> Pin<Box<dyn Future<Output = HandlerService<Envelope>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.entered.notify_one();
            self.barrier.wait().await;
            service
        })
    }
}

#[tokio::test]
async fn cancelled_pair_startup_releases_listener() -> TestResult {
    let entered = Arc::new(Notify::new());
    let barrier = Arc::new(Barrier::new(2));
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let factory = {
        let factory_entered = Arc::clone(&entered);
        let factory_barrier = Arc::clone(&barrier);
        move || -> TestResult<wireframe::app::WireframeApp> {
            let app = wireframe::app::WireframeApp::<_, _, Envelope>::new()?
                .route(1, Arc::clone(&handler))?
                .wrap(PreparationBarrier {
                    entered: Arc::clone(&factory_entered),
                    barrier: Arc::clone(&factory_barrier),
                })?;
            Ok(app)
        }
    };

    let listener = unused_listener()?;
    let addr = listener.local_addr()?;

    {
        let startup = start_pair(factory, |builder| builder, listener);
        let mut pinned = std::pin::pin!(startup);
        let entered_wait = entered.notified();
        let first_poll = poll!(pinned.as_mut());
        if matches!(first_poll, Poll::Ready(_)) {
            return Err(crate::TestError::Msg(
                "startup must still be preparing at the first poll".into(),
            ));
        }
        tokio::time::timeout(Duration::from_secs(1), entered_wait)
            .await
            .map_err(|_| crate::TestError::Msg("preparation never blocked".into()))?;
        // Leaving the block drops the pinned future, and with it the bound
        // server, without the barrier ever being crossed.
    }

    wait_for_listener_release(addr).await
}
