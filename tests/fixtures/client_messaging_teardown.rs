//! Test-only teardown helpers and regressions for the messaging world.
//!
//! Split from `client_messaging` to keep that module within the line limit. A
//! child module still reaches the world's private fields.

use super::*;
use crate::fixtures::teardown_probe::{
    assert_teardown_awaits,
    assert_waited_for_cancellation,
    spawn_panicking_task,
    spawn_slow_cancel_task,
};

/// Replace the world's server with a task that panics once it starts.
///
/// Teardown tests use this to show that a panicking server is reported rather
/// than discarded.
pub(crate) fn install_panicking_server(world: &mut ClientMessagingWorld) {
    let runtime = world.runtime.handle().clone();
    world.server = Some(ServerTaskGuard::new(spawn_panicking_task(&runtime)));
}

/// A panicking server must be reported, not mistaken for a clean abort.
#[test]
fn abort_server_reports_a_panicking_server() {
    let mut world = client_messaging_world().expect("build world");
    install_panicking_server(&mut world);

    let runtime = world.handle();
    let outcome = runtime.block_on(world.abort_server());

    assert!(
        outcome.is_err(),
        "a panicking server must surface as a step error"
    );
}

/// Teardown must await the abort, not merely request one.
///
/// The probe task runs on a runtime that outlives the world, so only an
/// awaited abort can make teardown block for the cancellation delay.
#[test]
fn drop_awaits_the_server_abort() {
    let probe_runtime = tokio::runtime::Runtime::new().expect("probe runtime");
    let mut world = client_messaging_world().expect("build world");
    world.server = Some(ServerTaskGuard::new(spawn_slow_cancel_task(
        probe_runtime.handle(),
    )));

    assert_teardown_awaits(|| drop(world));
    drop(probe_runtime);
}

/// Starting a second server must reap the first, not abandon it.
#[test]
fn start_echo_server_reaps_the_previous_server() {
    let probe_runtime = tokio::runtime::Runtime::new().expect("probe runtime");
    let mut world = client_messaging_world().expect("build world");
    world.server = Some(ServerTaskGuard::new(spawn_slow_cancel_task(
        probe_runtime.handle(),
    )));

    let runtime = world.handle();
    let started = std::time::Instant::now();
    let outcome = runtime.block_on(world.start_echo_server());
    let elapsed = started.elapsed();

    assert!(
        outcome.is_ok(),
        "starting a second server must succeed: {outcome:?}"
    );
    assert_waited_for_cancellation(elapsed);
    drop(probe_runtime);
}

/// A panicking previous server must be reported, not replaced silently.
#[test]
fn start_echo_server_reports_a_panicking_previous_server() {
    let mut world = client_messaging_world().expect("build world");
    install_panicking_server(&mut world);

    let runtime = world.handle();
    let outcome = runtime.block_on(world.start_echo_server());

    assert!(
        outcome.is_err(),
        "a panicking previous server must surface as a step error"
    );
}

/// Starting the echo server twice must leave only the second one running.
///
/// Unlike the probe tests, this drives the real server: the first listener must
/// be closed and its task gone by the time the second start returns.
#[test]
fn starting_twice_replaces_the_first_server() {
    let mut world = client_messaging_world().expect("build world");
    let runtime = world.handle();
    let baseline = runtime.metrics().num_alive_tasks();

    runtime
        .block_on(world.start_echo_server())
        .expect("first start");
    let first_addr = world.addr.expect("first server address");
    runtime
        .block_on(world.start_echo_server())
        .expect("second start");

    let first_still_listening = runtime
        .block_on(tokio::net::TcpStream::connect(first_addr))
        .is_ok();
    assert!(
        !first_still_listening,
        "the first server must be reaped before the second starts"
    );
    assert_eq!(
        runtime.metrics().num_alive_tasks(),
        baseline + 1,
        "only the second server may remain alive"
    );
}
