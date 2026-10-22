//! One logical pool slot: one physical socket plus admission permits.

use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use bb8::{Pool, PooledConnection};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

#[cfg(test)]
use super::test_support::DropProbe;
use super::{manager::WireframeConnectionManager, sync::lock_or_recover};
use crate::{client::ClientError, serializer::Serializer};

/// One physical socket slot backed by a `bb8` pool of size one.
pub(crate) struct PoolSlot<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Size-one pool containing the reusable physical client.
    pub(crate) pool: Pool<WireframeConnectionManager<S, P, C>>,
    /// Admission semaphore limiting concurrent logical operations.
    permits: Arc<Semaphore>,
    /// Reserves the socket for one lease's outstanding exchange.
    ///
    /// Admission permits bound concurrency; this gate is what makes a
    /// split-phase reply belong to a single lease, because a lease keeps it
    /// held between `send` and `receive`.
    protocol_gate: Arc<Semaphore>,
    /// Idle age at which the physical resource must be replaced.
    idle_timeout: Duration,
    /// Last healthy return time used for idle recycling.
    last_returned_at: Mutex<Option<Instant>>,
    /// Set when a lease left this socket unusable for anyone else.
    discard_on_next_checkout: AtomicBool,
    /// Test-only witness that a by-value slot is released with its core.
    #[cfg(test)]
    pub(super) drop_probe: Option<DropProbe>,
}

impl<S, P, C> PoolSlot<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Construct a slot with independent admission and recycle state.
    pub(crate) fn new(
        pool: Pool<WireframeConnectionManager<S, P, C>>,
        max_in_flight_per_socket: usize,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            pool,
            permits: Arc::new(Semaphore::new(max_in_flight_per_socket)),
            protocol_gate: Arc::new(Semaphore::new(1)),
            idle_timeout,
            last_returned_at: Mutex::new(None),
            discard_on_next_checkout: AtomicBool::new(false),
            #[cfg(test)]
            drop_probe: None,
        }
    }

    /// Try to reserve capacity without waiting or registering a waiter.
    pub(crate) fn try_acquire_permit(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.permits).try_acquire_owned().ok()
    }

    /// Wait for capacity; semaphore closure is surfaced as disconnection.
    pub(crate) async fn acquire_permit(&self) -> Result<OwnedSemaphorePermit, ClientError> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| ClientError::disconnected())
    }

    /// Close the admission gate so waiting acquisitions report disconnection.
    ///
    /// Nothing closes this gate in production, so the mapping from a closed
    /// semaphore to `ClientError::disconnected` is only reachable from tests.
    /// The switch lives beside the field it controls so the recorder test can
    /// drive the pool's real classification rather than a copy of it.
    #[cfg(test)]
    pub(super) fn close_admission(&self) { self.permits.close(); }

    /// Reserve this socket for one lease's outstanding exchange.
    pub(crate) async fn acquire_protocol_permit(
        &self,
    ) -> Result<OwnedSemaphorePermit, ClientError> {
        Arc::clone(&self.protocol_gate)
            .acquire_owned()
            .await
            .map_err(|_| ClientError::disconnected())
    }

    /// Check out a healthy connection, refreshing an idle one before use.
    ///
    /// `pending_responses` is the checking-out lease's unread-reply count. An
    /// idle socket is only replaced when that count is zero: a lease that
    /// already owns a reply must keep the very socket that reply will arrive
    /// on, or the pending response would be lost with the old connection.
    pub(crate) async fn checkout<'a>(
        &'a self,
        pending_responses: &'a AtomicUsize,
    ) -> Result<SlotConnection<'a, S, P, C>, ClientError> {
        let mut connection = self.get_connection().await?;

        let discard_requested = self.should_discard_lazily();
        // A lease holding an unread reply must keep the socket that reply will
        // arrive on, so its pending count wins over the idle-recycle check.
        let is_idle_expired =
            pending_responses.load(Ordering::Acquire) == 0 && self.should_recycle_idle();

        if discard_requested || is_idle_expired {
            connection.mark_broken();
            drop(connection);
            // The replacement checkout re-establishes the timestamp; clearing
            // it first keeps a failed replacement from recycling the next
            // socket on the strength of a stale value.
            self.clear_last_returned_at();
            connection = self.get_connection().await?;
        }

        Ok(SlotConnection {
            connection,
            armed: true,
            last_returned_at: &self.last_returned_at,
            pending_responses,
        })
    }

    /// Retire the socket on the next checkout because an earlier lease left it
    /// unsafe to reuse.
    pub(crate) fn discard_next_checkout(&self) {
        self.discard_on_next_checkout.store(true, Ordering::Release);
    }

    /// Claim the one-shot discard requested by an earlier lease.
    fn should_discard_lazily(&self) -> bool {
        self.discard_on_next_checkout.swap(false, Ordering::AcqRel)
    }

    /// Obtain the bb8 resource and translate checkout failures to client errors.
    async fn get_connection(
        &self,
    ) -> Result<PooledConnection<'_, WireframeConnectionManager<S, P, C>>, ClientError> {
        self.pool.get().await.map_err(|err| match err {
            bb8::RunError::User(error) => error,
            bb8::RunError::TimedOut => ClientError::from(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "pooled connection checkout timed out",
            )),
        })
    }

    /// Determine whether the last healthy return exceeded the idle lifetime.
    fn should_recycle_idle(&self) -> bool {
        self.lock_last_returned_at()
            .as_ref()
            .is_some_and(|returned_at| returned_at.elapsed() >= self.idle_timeout)
    }

    /// Clear the timestamp while replacing or invalidating the physical socket.
    fn clear_last_returned_at(&self) { *self.lock_last_returned_at() = None; }

    /// Recover the timestamp mutex after a poisoned test or worker thread.
    fn lock_last_returned_at(&self) -> MutexGuard<'_, Option<Instant>> {
        lock_or_recover(&self.last_returned_at)
    }
}

/// Borrowed checkout that records whether its physical resource stayed healthy.
pub(crate) struct SlotConnection<'a, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Checked-out resource whose drop records healthy or broken return state.
    connection: PooledConnection<'a, WireframeConnectionManager<S, P, C>>,
    /// Invalidates the resource unless an operation completed while checked out.
    armed: bool,
    /// Slot timestamp updated when the checkout is returned.
    last_returned_at: &'a Mutex<Option<Instant>>,
    /// Lease-owned unread-reply count cleared when cancellation invalidates it.
    pending_responses: &'a AtomicUsize,
}

impl<S, P, C> SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Keep the connection after its operation returned.
    pub(crate) fn disarm(&mut self) { self.armed = false; }

    /// Invalidate the connection while an operation is still holding it.
    pub(crate) fn mark_broken(&mut self) { self.connection.mark_broken(); }
}

impl<S, P, C> Deref for SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    type Target = super::managed::ManagedClientConnection<S, C>;

    fn deref(&self) -> &Self::Target { &self.connection }
}

impl<S, P, C> DerefMut for SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.connection }
}

impl<S, P, C> Drop for SlotConnection<'_, S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) {
        if self.armed {
            // The operation future was dropped before it completed, so the
            // exchange may still be half-written or half-read. A late reply
            // must never reach the next lease.
            self.connection.mark_broken();
            self.pending_responses.store(0, Ordering::Release);
            crate::metrics::inc_pool_incomplete_operation_discard();
            tracing::debug!(
                action = "discard_connection",
                pool_operation = "client_operation",
                reason = "operation_incomplete",
                "discarding pooled client connection after an incomplete operation"
            );
        }

        let mut last_returned_at = lock_or_recover(self.last_returned_at);

        if self.connection.is_broken() {
            *last_returned_at = None;
        } else {
            *last_returned_at = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for slot admission permits and idle-recycle bookkeeping.

    use std::{
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use googletest::{gtest, prelude::*};
    use tokio::{runtime::Builder, time::Instant};
    use tracing::Level;
    use wireframe_testing::ObservabilityHandle;

    use super::{
        super::test_support::{CaptureWriter, test_pool_slot},
        lock_or_recover,
    };

    /// The synchronous fast path must hand out a permit while capacity
    /// remains and refuse once it is exhausted; forcing `None` would push
    /// every acquire onto the waiting path.
    #[tokio::test]
    async fn try_acquire_permit_reflects_capacity() {
        let slot = test_pool_slot(1, Duration::from_secs(30));

        let held = slot.try_acquire_permit();
        assert!(held.is_some(), "fresh slot must yield an immediate permit");
        assert!(
            slot.try_acquire_permit().is_none(),
            "exhausted slot must refuse an immediate permit"
        );

        drop(held);
        assert!(
            slot.try_acquire_permit().is_some(),
            "released capacity must be immediately reusable"
        );
    }

    /// Clearing the idle timestamp must reset the recycle decision; leaving
    /// a stale timestamp would recycle the freshly created connection on the
    /// next checkout after a recycle whose lease never sets a new timestamp
    /// (for example when the replacement checkout fails).
    #[tokio::test]
    async fn clear_last_returned_at_resets_recycle_decision() {
        let slot = test_pool_slot(1, Duration::from_millis(1));

        *slot.lock_last_returned_at() = Some(tokio::time::Instant::now() - Duration::from_mins(1));
        assert!(
            slot.should_recycle_idle(),
            "stale timestamp must trigger a recycle"
        );

        slot.clear_last_returned_at();
        assert!(
            slot.lock_last_returned_at().is_none(),
            "clear must remove the timestamp"
        );
        assert!(
            !slot.should_recycle_idle(),
            "cleared timestamp must not trigger a recycle"
        );
    }

    #[gtest]
    fn idle_recycle_bookkeeping_recovers_after_timestamp_poison() {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("slot poison-recovery test runtime should build");
        let _runtime_guard = runtime.enter();
        let slot = Arc::new(test_pool_slot(1, Duration::from_millis(1)));
        let stale_timestamp = Instant::now() - Duration::from_mins(1);

        let poisoned_slot = Arc::clone(&slot);
        let join_result = thread::spawn(move || {
            let mut last_returned_at = poisoned_slot.lock_last_returned_at();
            *last_returned_at = Some(stale_timestamp);
            panic!("panic after slot timestamp update while holding the lock");
        })
        .join();

        expect_that!(join_result, err(anything()));
        expect_that!(slot.last_returned_at.is_poisoned(), eq(true));

        let captured_logs = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(Level::WARN)
            .with_writer(CaptureWriter::new(Arc::clone(&captured_logs)))
            .finish();
        let mut observability = ObservabilityHandle::new();
        metrics::with_local_recorder(observability.recorder(), || {
            tracing::subscriber::with_default(subscriber, || {
                expect_that!(slot.should_recycle_idle(), eq(true));
                slot.clear_last_returned_at();
                expect_that!(slot.should_recycle_idle(), eq(false));
            });
        });
        observability.snapshot();
        let log_bytes = lock_or_recover(&captured_logs).clone();
        let logs = String::from_utf8_lossy(&log_bytes);

        expect_that!(
            logs.as_ref(),
            contains_substring("recovering poisoned client pool bookkeeping lock")
        );
        // Poison remains set, so both recycle checks and the clear record recovery.
        expect_that!(
            observability
                .counter_without_labels(crate::metrics::POOL_BOOKKEEPING_POISON_RECOVERIES),
            eq(3)
        );
    }
}
