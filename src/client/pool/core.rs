//! Fixed physical slot storage and pool-wide lifecycle state.

use std::{
    future::Future,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::{Notify, OwnedSemaphorePermit};

use super::slot::PoolSlot;
use crate::{
    client::ClientError,
    metrics::{PoolAcquisitionMode, PoolAcquisitionOutcome, PoolPermitRaceGuard},
    serializer::Serializer,
};

/// Result of selecting a slot while retaining its independent admission token.
pub(crate) type SlotPermit = (usize, OwnedSemaphorePermit);

/// One shared root for the pool's fixed physical slots and shutdown state.
pub(crate) struct PoolCore<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Physical slots remain at stable addresses for the core's lifetime.
    slots: Box<[PoolSlot<S, P, C>]>,
    /// Cursor used to rotate the first slot attempted by each acquisition.
    next_slot: AtomicUsize,
    /// Release flag observed by blocked acquisitions and close.
    shutdown: AtomicBool,
    /// Notification used to cancel scheduler waits during shutdown.
    shutdown_notify: Notify,
    /// Test-only witness that fixed slot storage is released with the core.
    #[cfg(test)]
    pub(super) drop_probe: Option<super::test_support::DropProbe>,
}

impl<S, P, C> PoolCore<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Build a core around a fixed, non-growing slot allocation.
    pub(crate) fn new(slots: Box<[PoolSlot<S, P, C>]>) -> Self {
        Self {
            slots,
            next_slot: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            #[cfg(test)]
            drop_probe: None,
        }
    }

    /// Resolve an internal slot index without trusting the caller's value.
    pub(crate) fn slot(&self, slot_index: usize) -> Option<&PoolSlot<S, P, C>> {
        self.slots.get(slot_index)
    }

    /// Read the release-acquire shutdown flag used by all waiters.
    pub(crate) fn is_shutdown(&self) -> bool { self.shutdown.load(Ordering::Acquire) }

    /// Report how many physical slots this core owns.
    ///
    /// Available in test builds and through `test-support` so behavioural
    /// tests can prove they exercised a multi-slot pool.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub(crate) fn pool_size(&self) -> usize { self.slots.len() }

    /// Wait until pool shutdown wakes a cancellation-sensitive operation.
    pub(crate) async fn shutdown_notified(&self) {
        self.shutdown_notified_after_check(std::future::ready(()))
            .await;
    }

    /// Share the shutdown-check boundary with a deterministic interleaving test.
    async fn shutdown_notified_after_check(&self, after_check: impl Future<Output = ()>) {
        // Tokio records `notify_waiters` calls made after this future is
        // created, even before its first poll.
        let notified = self.shutdown_notify.notified();
        if self.is_shutdown() {
            return;
        }
        after_check.await;
        notified.await;
    }

    /// Publish shutdown and wake scheduler-managed permit waits.
    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.shutdown_notify.notify_waiters();
    }

    /// Select the first available slot in rotation order without slot Arcs.
    ///
    /// The attempt is recorded with its bounded outcome so pool saturation and
    /// shutdown are visible without adding per-slot or caller identifiers.
    pub(crate) fn try_acquire_permit(&self) -> Option<SlotPermit> {
        let outcome = self.try_acquire_permit_inner();
        let outcome_label = match &outcome {
            Some(_) => PoolAcquisitionOutcome::Granted,
            None if self.is_shutdown() => PoolAcquisitionOutcome::Shutdown,
            None => PoolAcquisitionOutcome::Exhausted,
        };
        crate::metrics::inc_pool_acquisition(PoolAcquisitionMode::Immediate, outcome_label);
        outcome
    }

    /// Try each slot in rotation order, reporting only the permit result.
    fn try_acquire_permit_inner(&self) -> Option<SlotPermit> {
        if self.is_shutdown() {
            return None;
        }

        let selected = self.ordered_slot_indices().find_map(|slot_index| {
            self.slot(slot_index)?
                .try_acquire_permit()
                .map(|permit| (slot_index, permit))
        });

        if self.is_shutdown() {
            drop(selected);
            None
        } else {
            selected
        }
    }

    /// Probe rotated slots before racing borrowed futures over blocked slots.
    ///
    /// A free slot at the rotated starting point wins before any waiters are
    /// registered. If every slot is full, the race futures borrow the stable
    /// slot storage; when polled, each future registers one FIFO waiter on its
    /// semaphore. The serial service worker runs at most one race at a time for
    /// the pool scheduler.
    pub(crate) async fn acquire_slot_permit(&self) -> Result<SlotPermit, ClientError> {
        let mut race = None;
        let result = self.acquire_slot_permit_inner(&mut race).await;
        let outcome = match &result {
            Ok(_) => PoolAcquisitionOutcome::Granted,
            Err(_) if self.is_shutdown() || self.slots.is_empty() => {
                PoolAcquisitionOutcome::Shutdown
            }
            Err(_) => PoolAcquisitionOutcome::Error,
        };
        crate::metrics::inc_pool_acquisition(PoolAcquisitionMode::PermitRace, outcome);
        if let Some(race) = race {
            race.finish(outcome);
        }
        result
    }

    /// Try rotated slots before registering blocked semaphore waiters.
    async fn acquire_slot_permit_inner<'a>(
        &'a self,
        race: &mut Option<PoolPermitRaceGuard<'a>>,
    ) -> Result<SlotPermit, ClientError> {
        if self.is_shutdown() || self.slots.is_empty() {
            return Err(ClientError::disconnected());
        }

        let ordered_indices = self.ordered_slot_indices();
        let selected = ordered_indices.clone().find_map(|slot_index| {
            self.slot(slot_index)?
                .try_acquire_permit()
                .map(|permit| (slot_index, permit))
        });

        let (slot_index, permit) = if let Some(selected) = selected {
            selected
        } else {
            *race = Some(PoolPermitRaceGuard::start(&self.shutdown));
            self.acquire_from_blocked_slots(ordered_indices).await?
        };

        if self.is_shutdown() {
            drop(permit);
            Err(ClientError::disconnected())
        } else {
            Ok((slot_index, permit))
        }
    }

    /// Race borrowed FIFO acquisitions after every immediate probe fails.
    async fn acquire_from_blocked_slots(
        &self,
        ordered_indices: impl Iterator<Item = usize>,
    ) -> Result<SlotPermit, ClientError> {
        // FuturesUnordered does not promise poll order, so the caller probes
        // first to make a free rotated starting slot deterministic. Each
        // future borrows the core's stable slots. Dropping the stream cancels
        // losing semaphore acquisitions and releases any assigned permits.
        let mut pending = FuturesUnordered::new();
        for slot_index in ordered_indices {
            let slot = self
                .slot(slot_index)
                .ok_or_else(ClientError::disconnected)?;
            pending.push(async move {
                let permit = slot.acquire_permit().await?;
                Ok((slot_index, permit))
            });
        }
        let result = pending
            .next()
            .await
            .unwrap_or_else(|| Err(ClientError::disconnected()));
        drop(pending);
        result
    }

    /// Rotate slot indices once for the immediate probe and blocked race.
    ///
    /// The cursor is read once per attempt, and only for a non-empty pool, so
    /// startup rotation stays cheap and an empty pool consumes no cursor value.
    /// The seed is reduced below `len` before any offset is added: a wrapped
    /// `usize` is congruent to its value minus `2^word`, which is a multiple of
    /// `len` only for power-of-two pool sizes. Reducing after the addition
    /// would therefore repeat one slot and skip another in a three- or
    /// five-slot pool once the cursor overflowed.
    ///
    /// Each call advances the starting cursor exactly once, even when a slot
    /// is selected by the immediate probe.
    fn ordered_slot_indices(&self) -> impl Iterator<Item = usize> + Clone + '_ {
        let len = self.slots.len();
        let start = if len == 0 {
            0
        } else {
            rotation_start(len, self.next_slot.fetch_add(1, Ordering::Relaxed))
        };

        rotation_indices(len, start)
    }
}

/// Reduce a raw cursor into a valid first slot index for `len` slots.
///
/// The empty case reports zero so the rotation can be described without a
/// branch at every call site; no cursor value is consumed for an empty pool.
const fn rotation_start(len: usize, cursor: usize) -> usize {
    if len == 0 {
        0
    } else {
        cursor.wrapping_rem(len)
    }
}

/// Visit every slot index once, starting at `start` and wrapping once.
///
/// `start` must be below `len`, which [`rotation_start`] guarantees, so the
/// additions cannot overflow; `wrapping_rem` is the lint-permitted spelling of
/// the remainder rather than an overflow workaround here.
fn rotation_indices(len: usize, start: usize) -> impl Iterator<Item = usize> + Clone {
    (0..len).map(move |offset| start.wrapping_add(offset).wrapping_rem(len.max(1)))
}

#[cfg(test)]
#[path = "rotation_tests.rs"]
mod rotation_tests;

#[cfg(test)]
#[path = "core_tests/mod.rs"]
mod tests;
