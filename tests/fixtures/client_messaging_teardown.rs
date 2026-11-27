//! Test-only teardown helpers and regressions for the messaging world.
//!
//! Split from `client_messaging` to keep that module within the line limit. A
//! child module still reaches the world's private fields.

use tokio::sync::oneshot::error::TryRecvError;

use super::*;

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

/// Replace the world's server with a task that never finishes.
///
/// The returned receiver closes when that task is dropped, which is how the
/// teardown test observes that the abort was awaited.
pub(crate) fn install_probe_server(
    world: &mut ClientMessagingWorld,
) -> tokio::sync::oneshot::Receiver<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let runtime = world.handle();
    world.server = Some(ServerTaskGuard::new(runtime.spawn(async move {
        let _keep_alive = sender;
        std::future::pending::<()>().await;
    })));
    receiver
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

/// Teardown must await the abort, so the task is gone before the runtime.
#[test]
fn drop_reaps_the_server_before_the_runtime_is_dropped() {
    let mut world = client_messaging_world().expect("build world");
    let mut receiver = install_probe_server(&mut world);

    drop(world);

    assert!(
        matches!(receiver.try_recv(), Err(TryRecvError::Closed)),
        "teardown must await the abort, which drops the server task"
    );
}
