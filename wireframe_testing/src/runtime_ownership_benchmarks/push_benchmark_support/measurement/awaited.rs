//! The awaited send regimes: one that cannot wait, one that must, one that may.
//!
//! All three call the same `push_high_priority`/`push_low_priority` entry point;
//! what separates them is whether the queue has room. That difference is not
//! visible from the API's return value — an awaited send that parks returns
//! exactly what one that did not returns — so each function here states what it
//! expects of the wait counters and fails the measurement when the expectation
//! cannot be met. A scenario whose regime did not occur is not a slow
//! measurement of the right thing; it is a measurement of the wrong thing under
//! a misleading label.
//!
//! [`send_drained`] covers the two scenarios where a consumer is running and the
//! queue may or may not hold the whole run. There the wait count is *reported*,
//! not asserted: whether the producer outran its consumer is the observation,
//! so turning it into a failure would suppress the finding. The no-park claim
//! belongs to
//! [`PushScenario::UncontendedHigh`](super::super::scenario::PushScenario::UncontendedHigh),
//! whose workload validation makes parking impossible; the pass asserts that
//! impossibility held in practice.
//!
//! See [`PushWaitCounters`](wireframe::push::PushWaitCounters) for why the
//! existing `reported_queue_full` flag cannot stand in for these counters.

use std::time::{Duration, Instant};

use wireframe::push::{PushError, PushHandle, PushWaitCounters};

use super::{SendOutcome, build_queues, frame};
use crate::runtime_ownership_benchmarks::push_benchmark_support::scenario::{Lane, PushWorkload};

/// Send every frame in the workload's lane, classifying a closed queue as fatal.
///
/// A closed queue during a measurement means the fixture tore down underneath
/// the producer, which invalidates the sample: the frames after the close were
/// never sent at all, so reporting a duration for them would be a fabrication.
///
/// # Errors
///
/// Returns a description if the queue closed mid-run.
async fn send_all(
    handle: &PushHandle<u8>,
    workload: PushWorkload,
    lane: Lane,
) -> Result<(), String> {
    for index in 0..workload.sends {
        let frame = frame(index);
        let sent = match lane {
            Lane::High => handle.push_high_priority(frame).await,
            Lane::Low => handle.push_low_priority(frame).await,
        };
        if matches!(sent, Err(PushError::Closed)) {
            return Err(format!(
                "the queue closed after {index} of {} sends, so the run is not a valid sample",
                workload.sends
            ));
        }
    }
    Ok(())
}

/// Run an awaited scenario against a draining consumer.
///
/// The consumer is spawned before the first send, so it is already parked on
/// the queue when the producer starts. When `must_not_wait` is set the workload
/// has been validated to hold the whole run, so no send *can* park and a
/// non-zero wait count means the run did not measure what its label claims;
/// otherwise the count is reported as found, because whether the producer
/// outran its consumer is precisely what the scenario is asking.
///
/// The probe count is asserted in both cases. Without it a zero wait count
/// would be indistinguishable from a run that never reached the channel, and
/// the no-back-pressure claim would rest on nothing having been measured.
///
/// # Errors
///
/// Returns a description if the queue closed early, the consumer failed, the
/// frames did not all arrive, no send reached the channel, or — for a workload
/// that must not wait — a send parked anyway. That last case means the caller
/// must widen the queue rather than publish the timing.
pub(super) async fn send_drained(
    workload: PushWorkload,
    must_not_wait: bool,
) -> Result<SendOutcome, String> {
    let lane = workload.scenario.lane();
    let (queues, handle, _dlq_rx) = build_queues(workload)?;
    let mut draining = queues;

    let consumer = tokio::spawn(async move {
        let mut received = 0usize;
        while draining.recv().await.is_some() {
            received += 1;
        }
        received
    });

    let wait_before = handle.wait_counters();
    send_all(&handle, workload, lane).await?;
    let waited = handle.wait_counters().since(wait_before);

    // Drop the handle so the queue closes and the consumer finishes; otherwise
    // the join would wait for a producer that is still alive.
    drop(handle);
    let received = consumer
        .await
        .map_err(|err| format!("draining consumer failed: {err}"))?;

    if must_not_wait && waited.awaited_capacity_waits != 0 {
        return Err(format!(
            "scenario {} is the no-back-pressure control, but {} of {} sends had to wait for \
             capacity; the queue of capacity {} did not hold the run",
            workload.scenario.label(),
            waited.awaited_capacity_waits,
            workload.sends,
            workload.queue_capacity
        ));
    }
    if waited.capacity_probes == 0 {
        return Err(format!(
            "no send reached the channel, so the measured wait count of {} rests on nothing; \
             {received} frames were drained",
            waited.awaited_capacity_waits
        ));
    }
    if received != workload.sends {
        return Err(format!(
            "consumer drained {received} of {} frames, so frames were lost",
            workload.sends
        ));
    }

    Ok(SendOutcome {
        // An awaited send parks rather than failing when the queue is full, so
        // every send that did not observe a closed queue was accepted.
        accepted: workload.sends,
        drained: received,
        dead_lettered: 0,
        reported_queue_full: false,
        waited,
    })
}

/// Run the awaited back-pressure scenario: sends that must park for capacity.
///
/// The queue is deliberately narrower than the send count and **nothing drains
/// it until a producer has demonstrably parked**. That is the difference between
/// this scenario and the uncontended one: there, a consumer was already running
/// and the assertion was that nobody ever waited; here, the absence of a
/// consumer is what forces the wait, and the assertion is that the wait
/// happened.
///
/// The producer runs as its own task so this one can watch the queue's counters
/// without blocking on a send. The first
/// [`queue_capacity`](PushWorkload::queue_capacity) sends fill the queue; the
/// next parks, which the observer sees as a rise in
/// [`PushWaitCounters::awaited_capacity_waits`]. Draining then begins, and the
/// parked send completes. Waiting for the park is bounded, but the bound is not
/// load-bearing: the observer drains the queue whether or not it ever observed a
/// park, so a workload that failed to park finishes and is reported, rather than
/// hanging.
///
/// # Errors
///
/// Returns a description if the queue closed early, the producer or the drain
/// panicked, the frames did not all arrive, or the run never parked for
/// capacity. The last case means the scenario did not exercise awaited
/// back-pressure at all.
pub(super) async fn send_backpressure(workload: PushWorkload) -> Result<SendOutcome, String> {
    let lane = workload.scenario.lane();
    let (mut queues, handle, _dlq_rx) = build_queues(workload)?;

    let wait_before = handle.wait_counters();
    let producer_handle = handle.clone();
    let producer = tokio::spawn(async move { send_all(&producer_handle, workload, lane).await });

    // Watch for the park before draining anything, so the wait is observed
    // rather than inferred from the workload's shape. The producer is
    // independent of this task, so a failing observation costs a bounded delay
    // rather than a deadlock.
    let observed_wait = observe_parked_send(&handle, wait_before).await;

    // Drain only now, and in its own task so this one can await the producer.
    // Draining regardless of what was observed is what keeps a failed
    // observation a failed *scenario* rather than a hung one: the parked
    // producer is released and the run finishes so the failure can be reported.
    let drain = tokio::spawn(async move {
        let mut received = 0usize;
        while queues.recv().await.is_some() {
            received += 1;
        }
        received
    });

    producer
        .await
        .map_err(|err| format!("back-pressure producer failed: {err}"))??;
    let waited = handle.wait_counters().since(wait_before);
    // Dropping the last sender closes the queue, which is what ends the drain
    // loop; without this the join below would wait on a queue that never closes.
    drop(handle);
    let received = drain
        .await
        .map_err(|err| format!("back-pressure drain failed: {err}"))?;

    if observed_wait == 0 {
        return Err(format!(
            "no producer parked within {BACKPRESSURE_WAIT_TIMEOUT:?}, so this run did not \
             exercise awaited back-pressure; the queue of capacity {} outran a {}-send workload",
            workload.queue_capacity, workload.sends
        ));
    }
    if received != workload.sends {
        return Err(format!(
            "the drain received {received} of {} frames, so frames were lost",
            workload.sends
        ));
    }

    Ok(SendOutcome {
        // A parked send eventually completes rather than failing, so every send
        // that did not observe a closed queue was accepted.
        accepted: workload.sends,
        drained: received,
        dead_lettered: 0,
        // Awaited sends never report a full queue: they wait instead. This
        // being `false` is the point of the counters above, not a contradiction.
        reported_queue_full: false,
        waited,
    })
}

/// How long to watch for a producer to park before draining anyway.
///
/// This bounds the wait for the *observation*, not the producer: the drain that
/// follows runs either way, so a run that never parks is reported as a failed
/// scenario rather than hanging. Generous, because a loaded runtime can delay a
/// freshly spawned task well past a scheduler quantum and a false negative here
/// would fail a correct workload.
const BACKPRESSURE_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Watch until the producer's awaited sends have parked, or the budget expires.
///
/// Returns the wait count observed, which is zero if the producer never parked.
/// Polls with `yield_now` rather than sleeping: the producer is on the same
/// runtime, so yielding is what lets it run at all, and it reaches the parked
/// state in a handful of quanta.
async fn observe_parked_send(handle: &PushHandle<u8>, before: PushWaitCounters) -> u64 {
    let deadline = Instant::now() + BACKPRESSURE_WAIT_TIMEOUT;
    loop {
        let waited = handle.wait_counters().since(before);
        if waited.awaited_capacity_waits > 0 {
            return waited.awaited_capacity_waits;
        }
        if Instant::now() >= deadline {
            return 0;
        }
        tokio::task::yield_now().await;
    }
}
