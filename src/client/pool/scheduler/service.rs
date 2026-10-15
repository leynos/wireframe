//! Background servicing for queued client-pool acquisitions.

use std::sync::Arc;

use super::{
    super::{client_pool::ClientPoolInner, lease::PooledClientLease},
    PoolScheduler,
    WaiterSender,
};
use crate::{client::ClientError, serializer::Serializer};

impl<S, P, C> PoolScheduler<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Serially service waiters so fairness order is deterministic.
    pub(super) async fn service_waiters(self: Arc<Self>, inner: Arc<ClientPoolInner<S, P, C>>) {
        while let Some(sender) = self.take_next_waiter_or_stop() {
            self.service_one_waiter(sender, Arc::clone(&inner)).await;
        }
    }

    /// Race slot capacity against shutdown or cancellation for one waiter.
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! macro internally uses % for random branch selection"
    )]
    async fn service_one_waiter(
        &self,
        mut sender: WaiterSender<S, P, C>,
        inner: Arc<ClientPoolInner<S, P, C>>,
    ) {
        let result = tokio::select! {
            permit_result = inner.acquire_slot_permit() => {
                permit_result.map(|(slot, permit)| {
                    PooledClientLease::new(slot, permit, Some(Arc::clone(&inner)))
                })
            }
            () = inner.shutdown_notified() => {
                self.finish_waiter();
                drop(sender.send(Err(ClientError::disconnected())));
                return;
            }
            () = sender.closed() => {
                self.finish_waiter();
                return;
            }
        };

        self.finish_waiter();
        if let Err(send_result) = sender.send(result)
            && let Ok(lease) = send_result
        {
            drop(lease);
        }
    }
}
