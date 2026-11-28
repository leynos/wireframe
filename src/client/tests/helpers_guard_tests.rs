//! Regression tests for the guarded connection helper.
//!
//! Split from `helpers` to keep that module within the line limit.

use tokio::sync::oneshot::{self, error::TryRecvError};

use super::*;

/// A refused connection must still reap the accept task.
///
/// The accept task serves a different socket and never completes, so only the
/// guard's abort can end it. The channel closes when the task's future is
/// dropped, which proves the handle was awaited rather than detached.
#[tokio::test]
async fn connect_guarded_reaps_the_accept_task_when_the_connection_fails() {
    let (accept_listener, _accept_addr) = bind_loopback().await.expect("bind accept listener");
    let (sender, mut receiver) = oneshot::channel::<()>();
    let accept: AcceptHandle = tokio::spawn(async move {
        let _keep_alive = sender;
        accept_listener.accept().await.map(|(stream, _)| stream)
    });

    let (refused_listener, refused_addr) = bind_loopback().await.expect("bind refused address");
    drop(refused_listener);

    let outcome = connect_guarded_on(accept, refused_addr, |builder| builder).await;

    assert!(outcome.is_err(), "a refused connection must be reported");
    assert!(
        matches!(receiver.try_recv(), Err(TryRecvError::Closed)),
        "the accept task must be reaped when the connection fails"
    );
}
