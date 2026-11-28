//! Shared probe for scenario-teardown ordering tests.
//!
//! A teardown that awaits an abort and one that merely requests it both stop
//! the task eventually, so a closed channel cannot tell them apart. Making the
//! cancellation take measurable time and timing the teardown can: only the
//! awaited form blocks for it.

use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

/// Cancellation delay long enough to distinguish the two teardown forms.
pub(crate) const CANCELLATION_DELAY: Duration = Duration::from_millis(250);

/// Sleep while dropped, so cancelling this task takes [`CANCELLATION_DELAY`].
struct SlowCancel;

impl Drop for SlowCancel {
    fn drop(&mut self) { std::thread::sleep(CANCELLATION_DELAY); }
}

/// Spawn a parked task on `runtime` whose cancellation takes measurable time.
///
/// The task reports that it has started before parking, so the abort can only
/// land on a task whose future is already established and whose cancellation
/// therefore has to drop it. The returned runtime must outlive the teardown
/// under test.
pub(crate) fn spawn_slow_cancel_task(runtime: &tokio::runtime::Handle) -> JoinHandle<()> {
    let (started, has_started) = tokio::sync::oneshot::channel();
    let handle = runtime.spawn(async move {
        let _slow_cancel = SlowCancel;
        let _signalled = started.send(());
        std::future::pending::<()>().await;
    });
    has_started
        .blocking_recv()
        .expect("the probe task must start before teardown");
    handle
}

/// Spawn a task on `runtime` that panics once it has started.
///
/// The start is awaited, so a later abort cannot pre-empt the panic and turn
/// it into a clean cancellation.
pub(crate) fn spawn_panicking_task(runtime: &tokio::runtime::Handle) -> JoinHandle<()> {
    let (started, has_started) = tokio::sync::oneshot::channel();
    let handle = runtime.spawn(async move {
        let _signalled = started.send(());
        panic!("fixture server panicked");
    });
    has_started
        .blocking_recv()
        .expect("the fixture server task must start before it is reaped");
    handle
}

/// Assert that a teardown blocked for the probe task's cancellation.
///
/// A teardown that only requests the abort returns without waiting, so its
/// elapsed time stays well under the cancellation delay.
pub(crate) fn assert_waited_for_cancellation(elapsed: Duration) {
    assert!(
        elapsed >= CANCELLATION_DELAY.saturating_sub(Duration::from_millis(50)),
        "teardown must await the abort, not just request it (took {elapsed:?})"
    );
}

/// Time `teardown` and assert that it waited for the cancelled probe task.
pub(crate) fn assert_teardown_awaits(teardown: impl FnOnce()) {
    let started = Instant::now();
    teardown();
    assert_waited_for_cancellation(started.elapsed());
}
