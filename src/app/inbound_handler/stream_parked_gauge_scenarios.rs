//! Child-process scenarios for the parked-body gauge.
//!
//! Each test here is ignored in the ordinary run: the scheduler in
//! [`super`] spawns this test binary once per scenario so the process-wide
//! gauge is measured without a sibling test moving it, and so a handler that
//! panics on purpose keeps its output out of the gate log.

use super::*;

/// Full module path of these scenarios, as libtest names them.
pub(super) const MODULE: &str = module_path!();

/// Drive the normal ending: two parked messages drain before the loop returns.
///
/// The gauge must read two while both wait, one while the drain's first reply
/// is held back, and zero once both are answered and the connection ends.
#[tokio::test]
#[ignore = "driven in a child process by the_parked_gauge_returns_to_baseline_for_every_ending"]
#[expect(
    clippy::print_stdout,
    reason = "the parent reads the child's stdout as the proof that it ran"
)]
async fn child_normal_exit() -> TestResult {
    let snapshotter = install_recorder()?;
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gated = Arc::new(Notify::new());
    let gate_release = Arc::new(Notify::new());
    let app = scenario_app(
        holding_handler(Arc::clone(&started), Arc::clone(&release)),
        Arc::clone(&gated),
        Arc::clone(&gate_release),
    )?;
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(app).await?;

    open_body(&mut client, LIVE_ROUTE, LIVE_KEY, &started).await?;
    park_one(&mut client, FIRST_KEY, GATED_PAYLOAD).await?;
    park_one(&mut client, SECOND_KEY, b"second").await?;
    await_gauge(&snapshotter, 2.0).await?;
    println!("scenario={} parked=2", Ending::NormalExit.label());

    // Free the slot: the drain pops the gated message first and holds its
    // reply, which is what makes the intermediate value observable.
    release.notify_one();
    send_request(
        &mut client,
        LIVE_ROUTE,
        &continuation_frame_payload(LIVE_KEY.into(), 1.into(), b"end", true)?,
    )
    .await?;
    ensure(
        read_reply(&mut client, "the live reply").await? == b"live".to_vec(),
        "the live body must be answered before the parked messages",
    )?;
    bounded_for(
        STEP_WAIT,
        "the drain reaching its first reply",
        gated.notified(),
    )
    .await?;
    ensure(
        gauge_reads(&snapshotter, 1.0),
        format!(
            "one message is still parked while its reply is pending, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    println!("scenario={} drained_one=1", Ending::NormalExit.label());

    gate_release.notify_one();
    ensure(
        read_reply(&mut client, "the first parked reply").await? == b"done".to_vec(),
        "the gated parked message must be answered once released",
    )?;
    ensure(
        read_reply(&mut client, "the second parked reply").await? == b"done".to_vec(),
        "the second parked message must be answered",
    )?;
    await_gauge(&snapshotter, 0.0).await?;
    println!("scenario={} drained_both=0", Ending::NormalExit.label());

    SinkExt::<Bytes>::close(&mut client).await?;
    let joined = bounded_for(STEP_WAIT, "the connection ending", connection).await?;
    joined??;
    ensure(
        gauge_reads(&snapshotter, 0.0),
        "a returned connection must leave nothing parked",
    )?;
    Ok(())
}

/// Drive the cancelled ending: the task is aborted while a message is parked.
#[tokio::test]
#[ignore = "driven in a child process by the_parked_gauge_returns_to_baseline_for_every_ending"]
#[expect(
    clippy::print_stdout,
    reason = "the parent reads the child's stdout as the proof that it ran"
)]
async fn child_task_abort() -> TestResult {
    let snapshotter = install_recorder()?;
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = scenario_app(
        holding_handler(Arc::clone(&started), Arc::clone(&release)),
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    )?;
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(app).await?;

    open_body(&mut client, LIVE_ROUTE, LIVE_KEY, &started).await?;
    park_one(&mut client, FIRST_KEY, b"parked").await?;
    await_gauge(&snapshotter, 1.0).await?;
    println!("scenario={} parked=1", Ending::TaskAbort.label());

    connection.abort();
    let joined = bounded_for(STEP_WAIT, "the aborted task ending", connection).await?;
    ensure(
        joined.is_err_and(|error| error.is_cancelled()),
        "the connection task must end because it was cancelled",
    )?;
    ensure(
        gauge_reads(&snapshotter, 0.0),
        format!(
            "a cancelled connection must release what it parked, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    println!("scenario={} aborted=0", Ending::TaskAbort.label());
    Ok(())
}

/// Drive the unwinding ending: a streaming handler panics with a message parked.
#[tokio::test]
#[ignore = "driven in a child process by the_parked_gauge_returns_to_baseline_for_every_ending"]
#[expect(
    clippy::print_stdout,
    reason = "the parent reads the child's stdout as the proof that it ran"
)]
async fn child_handler_panic() -> TestResult {
    let snapshotter = install_recorder()?;
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = scenario_app(
        panicking_handler(Arc::clone(&started), Arc::clone(&release)),
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    )?;
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(app).await?;

    open_body(&mut client, LIVE_ROUTE, LIVE_KEY, &started).await?;
    park_one(&mut client, FIRST_KEY, b"parked").await?;
    await_gauge(&snapshotter, 1.0).await?;
    println!("scenario={} parked=1", Ending::HandlerPanic.label());

    release.notify_one();
    let joined = bounded_for(STEP_WAIT, "the panicking handler ending", connection).await?;
    ensure(
        joined.is_err_and(|error| error.is_panic()),
        "the handler panic must end the connection task",
    )?;
    ensure(
        gauge_reads(&snapshotter, 0.0),
        format!(
            "an unwinding connection must release what it parked, gauge reads {}",
            parked_gauge(&snapshotter)
        ),
    )?;
    println!("scenario={} panicked=0", Ending::HandlerPanic.label());
    Ok(())
}
