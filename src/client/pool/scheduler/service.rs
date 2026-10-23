//! Background servicing for queued client-pool acquisitions.

use std::sync::Arc;

use super::{
    super::{client_pool::PoolSchedulerControl, core::PoolCore, lease::PooledClientLease},
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
    pub(super) async fn service_waiters(
        self: Arc<Self>,
        core: Arc<PoolCore<S, P, C>>,
        control: Arc<PoolSchedulerControl<S, P, C>>,
    ) {
        // Once shutdown is published no acquisition can succeed. Stopping here
        // keeps `notify_shutdown` the authority that completes queued waiters;
        // the worker only resolves the one waiter it has already claimed.
        while !core.is_shutdown() {
            let Some(sender) = self.take_next_waiter_or_stop() else {
                return;
            };
            self.service_one_waiter(sender, Arc::clone(&core), Arc::clone(&control))
                .await;
        }
        self.stop_servicing();
    }

    /// Race slot capacity against shutdown or cancellation for one waiter.
    async fn service_one_waiter(
        &self,
        mut sender: WaiterSender<S, P, C>,
        core: Arc<PoolCore<S, P, C>>,
        control: Arc<PoolSchedulerControl<S, P, C>>,
    ) {
        let permit_race_core = Arc::clone(&core);
        let result = tokio::select! {
            permit_result = permit_race_core.acquire_slot_permit() => {
                permit_result.and_then(|(slot_index, permit)| {
                    if core.is_shutdown() {
                        drop(permit);
                        return Err(ClientError::disconnected());
                    }
                    PooledClientLease::new(
                        Arc::clone(&core),
                        slot_index,
                        permit,
                        Some(Arc::clone(&control)),
                    )
                    .ok_or_else(ClientError::disconnected)
                })
            }
            () = core.shutdown_notified() => {
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
