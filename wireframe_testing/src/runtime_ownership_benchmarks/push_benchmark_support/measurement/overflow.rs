//! The overflow send path: a non-blocking send against a full queue.
//!
//! Every scenario here fills the target queue to capacity and then never drains
//! it, so each send meets a full queue. That is the condition under test, and a
//! queue that failed to fill would silently measure the empty-queue path while
//! claiming to measure overflow — which is why the fill count is checked rather
//! than assumed.

use wireframe::push::{PushError, PushHandle};

use super::{SendOutcome, build_queues, frame};
use crate::runtime_ownership_benchmarks::push_benchmark_support::scenario::{Lane, PushWorkload};

/// Fill the target queue to capacity, returning how many frames fit.
async fn fill_to_capacity(
    workload: PushWorkload,
    handle: &PushHandle<u8>,
) -> Result<usize, String> {
    let lane = workload.scenario.lane();
    let mut filled = 0usize;
    for index in 0..workload.queue_capacity {
        let frame = frame(index);
        let sent = match lane {
            Lane::High => handle.push_high_priority(frame).await,
            Lane::Low => handle.push_low_priority(frame).await,
        };
        match sent {
            Ok(()) => filled += 1,
            Err(PushError::Closed) => {
                return Err("queue closed while filling to capacity".to_string());
            }
            // `QueueFull` is the expected stop; `PushError` is non-exhaustive,
            // so any future variant also stops here rather than being mistaken
            // for a successful fill. Stopping is safe because the caller checks
            // that the queue reached capacity.
            Err(_) => break,
        }
    }
    Ok(filled)
}

/// Run an overflow scenario against a full queue.
pub(super) async fn send_overflow(workload: PushWorkload) -> Result<SendOutcome, String> {
    let policy = workload.scenario.overflow_policy()?;
    let (queues, handle, mut dlq_rx) = build_queues(workload)?;
    // The queues are deliberately not drained: overflow is the scenario.
    let filled = fill_to_capacity(workload, &handle).await?;
    if filled < workload.queue_capacity {
        return Err(format!(
            "only {filled} of {} frames fit, so the queue never reached capacity",
            workload.queue_capacity
        ));
    }

    let priority = workload.scenario.lane().priority();
    let mut outcome = SendOutcome::default();
    for index in 0..workload.sends {
        match handle.try_push(frame(index), priority, policy) {
            // A drop policy routes the frame to the DLQ *and* reports success,
            // so an `Ok` here means "the send path completed", not "the frame
            // is queued". Which of the two happened is told apart below by the
            // DLQ count, not by this return value.
            Ok(()) => outcome.accepted += 1,
            Err(PushError::QueueFull) => outcome.reported_queue_full = true,
            Err(PushError::Closed) => {
                return Err("queue closed during overflow sends".to_string());
            }
            // Non-exhaustive enum: an unrecognized error is reported as a
            // diagnostic rather than being silently counted as a drop.
            Err(err) => return Err(format!("overflow send failed: {err}")),
        }
    }

    // Drain the DLQ without awaiting: routing is synchronous, so every
    // dead-lettered frame is already buffered. The preload frames are not
    // counted here — only what the overflow sends routed.
    while dlq_rx.try_recv().is_ok() {
        outcome.dead_lettered += 1;
    }
    drop(queues);
    Ok(outcome)
}
