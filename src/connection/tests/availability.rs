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

use rstest::{fixture, rstest};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::super::{
    ConnectionActor,
    ConnectionStateError,
    output::AvailableOutput,
    state::ActorState,
};
use crate::{
    NoProtocolError,
    push::{PushConfigError, PushQueues},
    response::FrameStream,
};

/// Output source installed for one shutdown-suppression case.
#[derive(Clone, Copy)]
enum InstalledOutput {
    /// A streaming response with no frames ready.
    Response,
    /// A multi-packet channel whose sender stays open.
    MultiPacket,
}

impl InstalledOutput {
    /// Availability the gate must report while the actor is active.
    const fn available_before_shutdown(self) -> AvailableOutput {
        match self {
            Self::Response => AvailableOutput::Response,
            Self::MultiPacket => AvailableOutput::MultiPacket,
        }
    }

    /// Failure message for the post-shutdown suppression assertion.
    const fn suppression_message(self) -> &'static str {
        match self {
            Self::Response => "shutdown should suppress the installed response output",
            Self::MultiPacket => "shutdown should suppress the installed multi-packet output",
        }
    }

    /// Install the case's source on the actor.
    ///
    /// `keep_alive` retains the multi-packet sender for the whole case, because
    /// a bounded channel's receiver stays open only while its sender lives.
    fn install(
        self,
        actor: &mut ConnectionActor<u8, NoProtocolError>,
        keep_alive: &mut Vec<mpsc::Sender<u8>>,
    ) -> Result<(), ConnectionStateError> {
        match self {
            Self::Response => {
                let stream: FrameStream<u8, NoProtocolError> = Box::pin(futures::stream::pending());
                actor.set_response(Some(stream))?;
            }
            Self::MultiPacket => {
                let (tx, rx) = mpsc::channel::<u8>(1);
                keep_alive.push(tx);
                actor.set_multi_packet(Some(rx))?;
            }
        }
        Ok(())
    }

    /// Build the actor state matching the installed source.
    fn actor_state(self) -> ActorState {
        match self {
            Self::Response => ActorState::new(true, false),
            Self::MultiPacket => ActorState::new(false, true),
        }
    }

    /// Whether the actor still retains the installed source.
    const fn is_retained(self, actor: &ConnectionActor<u8, NoProtocolError>) -> bool {
        match self {
            Self::Response => actor.active_output.is_response(),
            Self::MultiPacket => actor.active_output.is_multi_packet(),
        }
    }
}

/// Build an actor with open, empty push lanes and no active output.
///
/// The lanes only need to exist; these tests never poll them. Building the
/// actor through the public constructor keeps the setup identical to the
/// production path that computes availability.
#[fixture]
fn availability_actor() -> Result<ConnectionActor<u8, NoProtocolError>, PushConfigError> {
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

/// Shutdown suppresses the installed output while retaining it for cleanup.
#[rstest]
#[case::response(InstalledOutput::Response)]
#[case::multi_packet(InstalledOutput::MultiPacket)]
fn shutdown_suppresses_the_installed_output(
    #[case] installed: InstalledOutput,
    availability_actor: Result<ConnectionActor<u8, NoProtocolError>, PushConfigError>,
) {
    let mut actor = availability_actor.expect("actor should build");
    let mut keep_alive = Vec::new();
    installed
        .install(&mut actor, &mut keep_alive)
        .expect("output should install");
    let mut state = installed.actor_state();

    assert!(
        actor.high_rx.is_some(),
        "the high-priority lane must stay open before shutdown"
    );
    assert!(
        actor.low_rx.is_some(),
        "the low-priority lane must stay open before shutdown"
    );
    assert!(
        actor
            .active_output
            .available_output(state.is_shutting_down())
            == installed.available_before_shutdown(),
        "the installed output must be pollable before shutdown"
    );

    state.start_shutdown();

    assert!(
        installed.is_retained(&actor),
        "shutdown must not clear the installed output"
    );
    assert!(
        matches!(
            actor
                .active_output
                .available_output(state.is_shutting_down()),
            AvailableOutput::None
        ),
        "{}",
        installed.suppression_message()
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
