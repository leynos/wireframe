//! Driving one push workload and reporting its counters.
//!
//! The rate limiter is disabled in every scenario here: the default limiter is
//! a leaky bucket capped at 100 pushes per second, so leaving it enabled would
//! make the measured cost almost entirely limiter bookkeeping and sleeping
//! rather than anything about queue ownership.
//!
//! The two regimes live in [`uncontended`] and [`overflow`] because they are
//! genuinely different control flows — one awaits capacity, the other never
//! does — and a shared body would blur which branch a regression sits in.

mod overflow;
mod uncontended;

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use overflow::send_overflow;
use tokio::sync::mpsc;
use uncontended::send_uncontended;
use wireframe::push::{PushHandle, PushQueues};

use super::scenario::{PushMeasurement, PushWorkload};

/// Fallback frame value, used only if the modulo ever exceeds `u8` range.
///
/// Frames are plain `u8` values here: the push path is queue bookkeeping and
/// channel hand-off, so a payload buffer would add allocation noise to a
/// measurement that is not about payload handling. The actor and startup
/// scenarios, which do carry payloads, use
/// [`Envelope`](wireframe::app::Envelope).
const FALLBACK_FRAME: u8 = 0xcd;

/// Queues, producer handle, and dead-letter receiver for one measurement.
pub(super) type PushFixture = (PushQueues<u8>, PushHandle<u8>, mpsc::Receiver<u8>);

/// One iteration's outcome.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SendOutcome {
    /// Sends the push API reported as accepted.
    pub(super) accepted: usize,
    /// Frames a consumer received.
    pub(super) drained: usize,
    /// Frames observed on the dead-letter queue.
    pub(super) dead_lettered: usize,
    /// Whether any send reported a full queue.
    pub(super) reported_queue_full: bool,
}

/// Build queues sized for `workload` with rate limiting disabled and a DLQ.
///
/// Returns the queues, the producer handle, and the dead-letter receiver. The
/// DLQ is always attached so that a drop policy's routing is observable rather
/// than silently discarded.
pub(super) fn build_queues(workload: PushWorkload) -> Result<PushFixture, String> {
    let (dlq_tx, dlq_rx) = mpsc::channel(workload.sends.max(1));
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(workload.queue_capacity)
        .low_capacity(workload.queue_capacity)
        .unlimited()
        .dlq(Some(dlq_tx))
        .build()
        .map_err(|err| format!("failed to build push queues: {err}"))?;
    Ok((queues, handle, dlq_rx))
}

/// Frame value for one send, kept distinguishable for debug builds.
///
/// A mask rather than `%`: `clippy::integer-division-remainder-used` is denied
/// workspace-wide, and the only property the frames need is that consecutive
/// sends differ, which a power-of-two wrap gives without a division.
pub(super) fn frame(index: usize) -> u8 { u8::try_from(index & 0xff).unwrap_or(FALLBACK_FRAME) }

/// Attempt `iterations` of one push workload, reporting the last iteration.
///
/// # Errors
///
/// Returns a description of the invalid workload, queue-construction failure,
/// consumer failure, or a queue that never reached capacity. An overflow
/// scenario whose queue cannot fill would measure the empty-queue path while
/// claiming to measure overflow, so it is rejected instead.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::runtime_ownership_benchmarks::{
///     PushScenario,
///     PushWorkload,
///     measure_push,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_push(
///     PushWorkload {
///         scenario: PushScenario::UncontendedHigh,
///         queue_capacity: 64,
///         sends: 16,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.accepted, 16);
/// assert_eq!(measurement.drained, 16);
/// # Ok(())
/// # }
/// ```
pub async fn measure_push(
    workload: PushWorkload,
    iterations: u64,
) -> Result<PushMeasurement, String> {
    workload.validate()?;
    if iterations == 0 {
        return Ok(PushMeasurement {
            sends: 0,
            accepted: 0,
            drained: 0,
            dead_lettered: 0,
            reported_queue_full: false,
            elapsed: Duration::ZERO,
        });
    }

    let started = Instant::now();
    let mut outcome = SendOutcome::default();
    for _ in 0..iterations {
        outcome = if workload.scenario.is_overflow() {
            send_overflow(workload).await?
        } else {
            send_uncontended(workload).await?
        };
    }
    let elapsed = started.elapsed();

    Ok(PushMeasurement {
        sends: workload
            .sends
            .saturating_mul(usize::try_from(iterations).unwrap_or(usize::MAX))
            .try_into()
            .unwrap_or(u64::MAX),
        accepted: black_box(outcome.accepted),
        drained: black_box(outcome.drained),
        dead_lettered: black_box(outcome.dead_lettered),
        reported_queue_full: outcome.reported_queue_full,
        elapsed,
    })
}
