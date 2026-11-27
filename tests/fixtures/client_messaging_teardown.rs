//! Test-only teardown helpers and regressions for the messaging world.
//!
//! Split from `client_messaging` to keep that module within the line limit. A
//! child module still reaches the world's private fields.

use super::*;
use crate::fixtures::teardown_probe::{assert_teardown_awaits, spawn_slow_cancel_task};

/// Replace the world's server with a task that panics once it starts.
///
/// Teardown tests use this to show that a panicking server is reported rather
/// than discarded.
pub(crate) fn install_panicking_server(world: &mut ClientMessagingWorld) {
    let (started, has_started) = tokio::sync::oneshot::channel();
    let runtime = world.runtime.handle().clone();
    world.server = Some(ServerTaskGuard::new(runtime.spawn(async move {
        let _signalled = started.send(());
        panic!("fixture server panicked");
    })));
    runtime
        .block_on(has_started)
        .expect("the fixture server task must start");
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
