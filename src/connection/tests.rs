//! Unit tests for connection actor scheduling decisions.

use tokio_util::sync::CancellationToken;

use super::{ActorState, ConnectionActor, Event};
use crate::{NoProtocolError, push::PushQueues};

#[tokio::test]
async fn shutdown_wins_over_a_ready_high_priority_push() {
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("push queues should build");
    handle
        .push_high_priority(7)
        .await
        .expect("high-priority push should queue");

    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<u8, NoProtocolError> =
        ConnectionActor::new(queues, handle, None, shutdown.clone());
    shutdown.cancel();

    let event = actor.next_event(&ActorState::new(false, false)).await;

    assert!(
        matches!(event, Event::Shutdown),
        "shutdown should take precedence over a ready high-priority push"
    );
    assert_eq!(
        actor
            .high_rx
            .as_ref()
            .expect("high-priority queue should remain open")
            .len(),
        1,
        "shutdown precedence should leave the ready push queued"
    );
}
