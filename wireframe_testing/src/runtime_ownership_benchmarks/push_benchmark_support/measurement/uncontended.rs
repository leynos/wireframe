//! The uncontended send path: awaiting capacity in a queue being drained.
//!
//! A consumer must be running, otherwise the queue fills and the awaited send
//! measures back-pressure instead of the send path. That is the whole reason
//! this regime is separate from the overflow one.

use wireframe::push::PushError;

use super::{SendOutcome, build_queues, frame};
use crate::runtime_ownership_benchmarks::push_benchmark_support::scenario::{Lane, PushWorkload};

/// Run the uncontended scenario against a draining consumer.
pub(super) async fn send_uncontended(workload: PushWorkload) -> Result<SendOutcome, String> {
    let lane = workload.scenario.lane();
    let (queues, handle, _dlq_rx) = build_queues(workload)?;
    let mut draining = queues;

    // A consumer must be running, otherwise the queue fills and the awaited
    // send measures back-pressure instead of the send path.
    let consumer = tokio::spawn(async move {
        let mut received = 0usize;
        while draining.recv().await.is_some() {
            received += 1;
        }
        received
    });

    for index in 0..workload.sends {
        let frame = frame(index);
        let sent = match lane {
            Lane::High => handle.push_high_priority(frame).await,
            Lane::Low => handle.push_low_priority(frame).await,
        };
        if matches!(sent, Err(PushError::Closed)) {
            return Err("uncontended consumer closed the queue early".to_string());
        }
    }

    // Drop the handle so the queue closes and the consumer finishes; otherwise
    // the join would wait for a producer that is still alive.
    drop(handle);
    let received = consumer
        .await
        .map_err(|err| format!("draining consumer failed: {err}"))?;

    Ok(SendOutcome {
        // An awaited send parks rather than failing when the queue is full, so
        // every send that did not observe a closed queue was accepted. The
        // consumer's count is reported separately: for this scenario the two
        // should agree, and a disagreement means frames were lost.
        accepted: workload.sends,
        drained: received,
        dead_lettered: 0,
        reported_queue_full: false,
    })
}
