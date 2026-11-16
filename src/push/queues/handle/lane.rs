//! Posting one frame onto a push lane under capacity and rate bounds.
//!
//! Both awaited push helpers funnel through `push_with_priority`, which
//! reserves queue capacity before it draws a rate permit and never awaits the
//! limiter directly. Keeping that machinery beside the handle's public surface
//! rather than inside it leaves the API declaration readable on its own.

use std::{
    future::{Future, poll_fn},
    time::Duration,
};

use leaky_bucket::RateLimiter;
use log::debug;
use tokio::{sync::mpsc, time::sleep};

use super::{
    super::{FrameLike, PushError, PushPriority, waits::record_first_pending},
    PushHandle,
};

impl<F: FrameLike> PushHandle<F> {
    /// Internal helper to push a frame with the requested priority.
    ///
    /// Capacity is reserved before a rate permit is drawn. A permit taken
    /// first would let several producers park holding one each, and those
    /// parked frames would then enter the queue back-to-back as soon as the
    /// consumer drained it, exceeding the configured rate on the wire. Holding
    /// a reservation instead delays only the producers that have nowhere to
    /// put a frame, and every frame still enters the queue at the configured
    /// rate.
    ///
    /// The limiter is never awaited directly: awaiting it registers this task
    /// as a waiter and reserves a token even if the future is never polled
    /// again (for example after a `now_or_never()` probe in tests), which can
    /// starve other pushes. A non-blocking check in a short sleep loop keeps
    /// tokens available to actively polled tasks.
    pub(super) async fn push_with_priority(
        &self,
        frame: F,
        priority: PushPriority,
    ) -> Result<(), PushError> {
        self.check_frame_len(&frame)?;
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };

        // A closed connection ends the producer's work, so report that rather
        // than waiting for the rate limiter to refill first.
        if tx.is_closed() {
            return Err(PushError::Closed);
        }

        // Reserve a slot first: awaiting capacity if the queue is currently
        // full, and surfacing `Closed` if the receiver has been dropped.
        //
        // The non-blocking probe below is what makes "did this push wait?"
        // observable. It reserves a slot if one is free, so the common case
        // still completes without parking. Probing on every send costs one
        // atomic acquisition, which is noise beside the channel operations it
        // guards, and it keeps the counter meaningful for any caller rather
        // than only for instrumentation that opted in.
        //
        // A refusal is *not* by itself a wait. It says one poll of `reserve`
        // would park, but nothing has polled yet, and a caller that frees a
        // slot in between gets a reservation that never suspends. Counting the
        // refusal would make the counter report a suspension that did not
        // happen — which is exactly what a back-pressure observer watching
        // this counter would then mistake for proof the producer had parked.
        // So the wait is recorded on the poll that actually returns `Pending`,
        // once per send, and only then. `reserve_capacity` is what watches for
        // that poll.
        self.0.waits.record_capacity_probe();
        let permit = match tx.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(())) => self.reserve_capacity(tx).await?,
            Err(mpsc::error::TrySendError::Closed(())) => return Err(PushError::Closed),
        };

        if let Some(ref limiter) = self.0.limiter {
            self.wait_for_permit(limiter, tx).await?;
        }
        permit.send(frame);
        debug!("frame pushed: priority={priority:?}");
        Ok(())
    }

    /// Interval between attempts to acquire a rate-limit permit.
    ///
    /// Kept short so tests that use `tokio::time::pause/advance` progress
    /// quickly while remaining negligible relative to the 1s refill window.
    const PERMIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

    /// Reserve a slot on a full channel, counting the wait only if it parks.
    ///
    /// The caller has already probed the channel and found it full, so this is
    /// the cold path. It polls the reservation future itself rather than
    /// awaiting it directly because only the poll result distinguishes the two
    /// outcomes a caller cares about: a slot freed before this future was
    /// first polled yields `Ready` and no wait, while a `Pending` is an actual
    /// suspension. The counter is incremented on the first `Pending` and then
    /// left alone: repetition would count one send once per wake-up.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] if the receiver was dropped before a slot
    /// could be reserved.
    async fn reserve_capacity<'a>(
        &self,
        sender: &'a mpsc::Sender<F>,
    ) -> Result<mpsc::Permit<'a, F>, PushError> {
        // Keep the caller's sender borrowed while polling so this slow path
        // does not create a second shared owner just to name the future.
        let mut reserve = std::pin::pin!(sender.reserve());
        let mut is_wait_recorded = false;
        poll_fn(|cx| {
            let polled = reserve.as_mut().poll(cx);
            record_first_pending(&polled, &mut is_wait_recorded, &self.0.waits);
            polled
        })
        .await
        .map_err(|_| PushError::Closed)
    }

    /// Poll permits without registering a parked waiter that could starve pushes.
    ///
    /// A closed channel is reported as such even while the limiter is
    /// exhausted, so a producer waiting for a refill does not outlive the
    /// connection it is pushing to.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] once the receiver has been dropped.
    async fn wait_for_permit(
        &self,
        limiter: &RateLimiter,
        sender: &mpsc::Sender<F>,
    ) -> Result<(), PushError> {
        loop {
            if sender.is_closed() {
                return Err(PushError::Closed);
            }
            if limiter.try_acquire(1) {
                return Ok(());
            }
            // The limiter is configured with a 1s refill interval; a short
            // sleep yields to the scheduler and advances virtual time in
            // tests (tokio::time::pause/advance).
            sleep(Self::PERMIT_POLL_INTERVAL).await;
        }
    }
}
