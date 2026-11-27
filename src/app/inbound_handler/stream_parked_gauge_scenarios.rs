//! Child-process scenarios for the parked-body gauge.
//!
//! Each test here is ignored in the ordinary run: the scheduler in
//! [`super`] spawns this test binary once per scenario so the process-wide
//! gauge is measured without a sibling test moving it, and so a handler that
//! panics on purpose keeps its output out of the gate log.

use super::{support::*, *};

/// Full module path of these scenarios, as libtest names them.
pub(super) const MODULE: &str = module_path!();

/// Drive the normal ending: two parked messages drain before the loop returns.
///
/// A sentinel connection keeps one message parked throughout, so the totals are
/// three while both test messages wait, two while the drain's first reply is
/// held back, and one once both are answered and the connection ends. Reading
/// one rather than zero is what proves the release happened exactly once: an
/// extra release would take the gauge below the sentinel.
#[tokio::test]
#[ignore = "driven in a child process by the_parked_gauge_returns_to_baseline_for_every_ending"]
#[expect(
    clippy::print_stdout,
    reason = "the parent reads the child's stdout as the proof that it ran"
)]
async fn child_normal_exit() -> TestResult {
    let snapshotter = install_recorder()?;
    let _sentinel = parked_sentinel().await?;
    await_gauge(&snapshotter, 1.0).await?;
    let mut paired = parked_pair(AfterRelease::Reply).await?;
    await_gauge(&snapshotter, 3.0).await?;
    println!("scenario={} parked=3", Ending::NormalExit.label());

    answer_live_body(&mut paired).await?;
    ensure(
        gauge_reads(&snapshotter, 2.0),
        format!(
            "two messages are still parked while a reply is pending, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    println!("scenario={} drained_one=2", Ending::NormalExit.label());

    answer_parked_pair(&mut paired).await?;
    await_gauge(&snapshotter, 1.0).await?;
    println!("scenario={} drained_both=1", Ending::NormalExit.label());

    SinkExt::<Bytes>::close(&mut paired.client).await?;
    let joined = bounded_for(STEP_WAIT, "the connection ending", paired.connection).await?;
    joined??;
    ensure(
        gauge_reads(&snapshotter, 1.0),
        format!(
            "a returned connection must leave only the sentinel parked, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    Ok(())
}

/// Drive the cancelled ending: the task is aborted while two messages are
/// parked, and the sentinel keeps the total visible.
#[tokio::test]
#[ignore = "driven in a child process by the_parked_gauge_returns_to_baseline_for_every_ending"]
#[expect(
    clippy::print_stdout,
    reason = "the parent reads the child's stdout as the proof that it ran"
)]
async fn child_task_abort() -> TestResult {
    let snapshotter = install_recorder()?;
    let _sentinel = parked_sentinel().await?;
    await_gauge(&snapshotter, 1.0).await?;
    let paired = parked_pair(AfterRelease::Reply).await?;
    await_gauge(&snapshotter, 3.0).await?;
    println!("scenario={} parked=3", Ending::TaskAbort.label());

    paired.connection.abort();
    let joined = bounded_for(STEP_WAIT, "the aborted task ending", paired.connection).await?;
    ensure(
        joined.is_err_and(|error| error.is_cancelled()),
        "the connection task must end because it was cancelled",
    )?;
    ensure(
        gauge_reads(&snapshotter, 1.0),
        format!(
            "a cancelled connection must release what it parked, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    println!("scenario={} aborted=1", Ending::TaskAbort.label());
    Ok(())
}

/// Drive the unwinding ending: a streaming handler panics while two messages
/// are parked, and the sentinel keeps the total visible.
#[tokio::test]
#[ignore = "driven in a child process by the_parked_gauge_returns_to_baseline_for_every_ending"]
#[expect(
    clippy::print_stdout,
    reason = "the parent reads the child's stdout as the proof that it ran"
)]
async fn child_handler_panic() -> TestResult {
    let snapshotter = install_recorder()?;
    let _sentinel = parked_sentinel().await?;
    await_gauge(&snapshotter, 1.0).await?;
    let paired = parked_pair(AfterRelease::Panic).await?;
    await_gauge(&snapshotter, 3.0).await?;
    println!("scenario={} parked=3", Ending::HandlerPanic.label());

    // Free the live handler, whose configured ending is to panic.
    paired.release.notify_one();
    let joined = bounded_for(STEP_WAIT, "the panicking handler ending", paired.connection).await?;
    ensure(
        joined.is_err_and(|error| error.is_panic()),
        "the handler panic must end the connection task",
    )?;
    ensure(
        gauge_reads(&snapshotter, 1.0),
        format!(
            "an unwinding connection must release what it parked, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    println!("scenario={} panicked=1", Ending::HandlerPanic.label());
    Ok(())
}
