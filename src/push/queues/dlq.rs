//! Dead-letter drop accounting for the push handle.
//!
//! A non-blocking push policy that discards a frame first tries to hand it to
//! a dead-letter queue. Only when *that* also fails — the dead-letter queue is
//! full or closed — is the frame genuinely lost, and that loss is what the
//! counter here records. The counter is deliberately narrow: a drop the
//! dead-letter queue accepted is not a lost frame and counts nothing.
//!
//! Because producers share one handle, the count and the throttled diagnostic
//! that flushes it are touched concurrently. [`DeadLetterState::route_or_count`]
//! carries the increment-and-maybe-log sequence, and the Loom models in
//! `crates/wireframe-loom/tests/push_dlq.rs` schedule it to check that the
//! reported and remaining drops always sum to the drops produced.

use std::time::{Duration, Instant};

use log::warn;
use tokio::sync::mpsc;

use super::{
    PushPolicy,
    PushPriority,
    sync::{AtomicUsize, Mutex, Ordering},
};

/// Why the dead-letter queue refused a frame.
///
/// The variant is logged in place of the frame itself: it tells an operator
/// how the queue failed without disclosing the payload.
#[derive(Clone, Copy, Debug)]
enum DlqState {
    /// The dead-letter queue exists but has no free capacity.
    Full,
    /// The dead-letter queue's receiver has been dropped.
    Closed,
}

/// Bounded description of a refused dead-letter send, safe to log.
///
/// This is the complete input to the loss diagnostic. It deliberately carries
/// no frame payload: the diagnostic must not disclose what was dropped.
#[derive(Clone, Copy)]
struct DropReport {
    /// Drops counted so far, including the one that triggered this report.
    dropped: usize,
    /// Priority of the queue that refused the frame.
    priority: PushPriority,
    /// Policy that requested the dead-letter hand-off.
    policy: PushPolicy,
    /// Why the dead-letter queue refused the frame.
    dlq_state: DlqState,
}

/// Shared dead-letter drop state: the pending count, the last log time, and
/// the cadence that decides when the diagnostic fires.
pub(crate) struct DeadLetterState<F> {
    /// Optional sink for frames discarded by non-blocking push policies.
    sink: Option<mpsc::Sender<F>>,
    /// Number of drops since the last diagnostic reset.
    drops: AtomicUsize,
    /// Timestamp used for interval-based drop logging.
    last_log: Mutex<Instant>,
    /// Running total of the drops the diagnostic has reported, kept only so
    /// Loom models can check that reported plus remaining equals produced.
    #[cfg(loom)]
    reported: AtomicUsize,
    /// Count interval used for drop diagnostics.
    log_every_n: usize,
    /// Time interval used for drop diagnostics.
    log_interval: Duration,
}

#[cfg(loom)]
impl<F> DeadLetterState<F> {
    /// Total drops the throttled diagnostic has reported so far.
    pub(crate) fn reported_count(&self) -> usize { self.reported.load(Ordering::SeqCst) }

    /// Drops counted but not yet reported by the diagnostic.
    pub(crate) fn pending_count(&self) -> usize { self.drops.load(Ordering::SeqCst) }
}

impl<F> DeadLetterState<F> {
    /// Wrap the dead-letter sender with zeroed counters and the given cadence.
    pub(crate) fn new(
        sink: Option<mpsc::Sender<F>>,
        log_every_n: usize,
        log_interval: Duration,
    ) -> Self {
        Self {
            sink,
            drops: AtomicUsize::new(0),
            last_log: Mutex::new(Instant::now()),
            #[cfg(loom)]
            reported: AtomicUsize::new(0),
            log_every_n,
            log_interval,
        }
    }

    /// Whether a dead-letter queue is configured to receive dropped frames.
    pub(crate) fn has_queue(&self) -> bool { self.sink.is_some() }

    /// Hand `frame` to the dead-letter queue, recording a loss if that fails.
    ///
    /// A frame the dead-letter queue accepts is not a lost frame and is not
    /// counted, whatever the caller's drop policy was. With no dead-letter
    /// queue configured there is nothing to fail to send to, so nothing is
    /// counted either.
    ///
    /// A refused frame is dropped here without being inspected. The resulting
    /// diagnostic records only bounded metadata — the dead-letter state,
    /// priority, policy and frame type — never the frame's contents, which may
    /// carry application payload or secrets.
    pub(crate) fn route_or_count(&self, frame: F, priority: PushPriority, policy: PushPolicy) {
        let Some(queue) = &self.sink else {
            return;
        };
        // The frame is lost either way, so this is where it leaves ownership:
        // neither failure variant is unwrapped and the payload is never
        // formatted.
        let dlq_state = match queue.try_send(frame) {
            Ok(()) => return,
            Err(mpsc::error::TrySendError::Full(_)) => DlqState::Full,
            Err(mpsc::error::TrySendError::Closed(_)) => DlqState::Closed,
        };
        let dropped = self.drops.fetch_add(1, Ordering::Relaxed) + 1;
        self.log_if_due(DropReport {
            dropped,
            priority,
            policy,
            dlq_state,
        });
    }

    /// Emit the diagnostic and reset the counter when the cadence says so.
    ///
    /// The warning describes why the triggering frame was lost and how often
    /// drops are reported; it never includes the frame's contents. The
    /// `trigger_*` fields describe only the frame that caused this report,
    /// while `dropped` aggregates every loss since the previous report.
    fn log_if_due(&self, report: DropReport) {
        let DropReport {
            dropped,
            priority,
            policy,
            dlq_state,
        } = report;
        let log_every_n = self.log_every_n;
        let log_interval = self.log_interval;
        let mut last_log = match self.last_log.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("DLQ last-log mutex poisoned; continuing with stale state");
                poisoned.into_inner()
            }
        };
        let should_log = (log_every_n != 0 && dropped.is_multiple_of(log_every_n))
            || last_log.elapsed() > log_interval;
        if !should_log {
            return;
        }
        // Take the count and report exactly what was taken. Reporting
        // `dropped` and then storing zero lost every increment another
        // producer made in between and reported drops a concurrent reset had
        // already cleared, so the log over-reported under contention.
        let reported = self.drops.swap(0, Ordering::Relaxed);
        *last_log = Instant::now();
        // A logger that loses the race to another producer's swap has
        // nothing left to report.
        if reported == 0 {
            return;
        }
        // Name the frame's type only. The payload is deliberately absent:
        // these logs may be shipped to aggregators with broader access than
        // the application's own DLQ consumer.
        //
        // `dropped` can aggregate losses from several priorities, policies or
        // dead-letter states, so the diagnostic fields are prefixed `trigger_`
        // to mark them as properties of the frame that fired this report.
        let frame_type = std::any::type_name::<F>();
        warn!(
            "DLQ dropped frames: trigger_dlq_state={dlq_state:?}, trigger_priority={priority:?}, \
             trigger_policy={policy:?}, frame_type={frame_type}, dropped={reported}, \
             log_every_n={log_every_n}, log_interval={log_interval:?}"
        );
        #[cfg(loom)]
        self.reported.fetch_add(reported, Ordering::Relaxed);
    }
}
