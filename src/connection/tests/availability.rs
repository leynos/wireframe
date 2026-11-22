//! Shutdown suppression for installed output sources.
//!
//! The observation loop must stop polling a streaming response or a
//! multi-packet channel once shutdown begins. `ConnectionActor` expresses that
//! rule in one place: [`ActiveOutput::available_output`] returns
//! [`AvailableOutput::None`] whenever the actor state reports shutdown, even
//! though the source itself stays installed for the cleanup path.
//!
//! These tests install each source, transition only [`ActorState`] to
//! shutdown, and assert the gate reports the source unavailable while the
//! source remains active. A regression makes an assertion fail deterministically
//! instead of leaving the observation loop armed until a mutation-testing
//! timeout notices.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::super::{ConnectionActor, output::AvailableOutput, state::ActorState};
use crate::{NoProtocolError, push::PushQueues, response::FrameStream};

/// Build an actor with open, empty push lanes and no active output.
///
/// The lanes only need to exist; these tests never poll them. Building the
/// actor through the public constructor keeps the setup identical to the
/// production path that computes availability.
fn availability_actor() -> Result<ConnectionActor<u8, NoProtocolError>, crate::push::PushConfigError>
{
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .build()?;
    Ok(ConnectionActor::new(
        queues,
        handle,
        None,
        CancellationToken::new(),
    ))
}

/// Shutdown suppresses an installed response stream without clearing it.
#[test]
fn shutdown_suppresses_the_installed_response_output() {
    let mut actor = availability_actor().expect("actor should build");
    let stream: FrameStream<u8, NoProtocolError> = Box::pin(futures::stream::pending());
    actor
        .set_response(Some(stream))
        .expect("response output should install");
    let mut state = ActorState::new(true, false);

    assert!(
        actor.high_rx.is_some(),
        "the high-priority lane must stay open before shutdown"
    );
    assert!(
        actor.low_rx.is_some(),
        "the low-priority lane must stay open before shutdown"
    );
    assert!(
        matches!(
            actor
                .active_output
                .available_output(state.is_shutting_down()),
            AvailableOutput::Response
        ),
        "an installed response must be pollable before shutdown"
    );

    state.start_shutdown();

    assert!(
        actor.active_output.is_response(),
        "shutdown must not clear the installed response"
    );
    assert!(
        matches!(
            actor
                .active_output
                .available_output(state.is_shutting_down()),
            AvailableOutput::None
        ),
        "shutdown should suppress the installed response output"
    );
    assert!(
        actor.high_rx.is_some(),
        "the high-priority lane must stay open after shutdown"
    );
    assert!(
        actor.low_rx.is_some(),
        "the low-priority lane must stay open after shutdown"
    );
}

/// Shutdown suppresses an installed multi-packet channel without clearing it.
#[test]
fn shutdown_suppresses_the_installed_multi_packet_output() {
    let mut actor = availability_actor().expect("actor should build");
    // Holding the sender open keeps the receiver installed rather than
    // closed, so only the shutdown gate can suppress the source.
    let (_tx, rx) = mpsc::channel::<u8>(1);
    actor
        .set_multi_packet(Some(rx))
        .expect("multi-packet output should install");
    let mut state = ActorState::new(false, true);

    assert!(
        actor.high_rx.is_some(),
        "the high-priority lane must stay open before shutdown"
    );
    assert!(
        actor.low_rx.is_some(),
        "the low-priority lane must stay open before shutdown"
    );
    assert!(
        matches!(
            actor
                .active_output
                .available_output(state.is_shutting_down()),
            AvailableOutput::MultiPacket
        ),
        "an installed multi-packet channel must be pollable before shutdown"
    );

    state.start_shutdown();

    assert!(
        actor.active_output.is_multi_packet(),
        "shutdown must not clear the installed multi-packet channel"
    );
    assert!(
        matches!(
            actor
                .active_output
                .available_output(state.is_shutting_down()),
            AvailableOutput::None
        ),
        "shutdown should suppress the installed multi-packet output"
    );
    assert!(
        actor.high_rx.is_some(),
        "the high-priority lane must stay open after shutdown"
    );
    assert!(
        actor.low_rx.is_some(),
        "the low-priority lane must stay open after shutdown"
    );
}
