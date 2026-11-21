//! Property coverage for per-connection protocol ownership.
//!
//! The example-based ownership tests name concrete connection counts. This
//! companion checks the same invariant for any small number of concurrently
//! open connections: every live connection adds exactly one strong reference to
//! the installed protocol, every teardown removes exactly one, and the
//! installed instance is dropped exactly once after the last reference is
//! released.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/recording_protocol.rs"]
mod recording_protocol;
#[path = "common/recording_protocol_ownership.rs"]
mod recording_protocol_ownership;

use proptest::{prelude::*, test_runner::TestCaseError};
use recording_protocol::{HookEvent, RecordingProtocolFixture};
use recording_protocol_ownership::{drop_count, is_alive, shared, strong_count, weak_handle};
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, ConnectionChannels},
    hooks::ProtocolHooks,
    push::PushQueues,
};
use wireframe_testing::TestResult;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    /// Each concurrently open connection owns exactly one reference.
    ///
    /// The fixture's reference is the baseline, so a connection owning a
    /// reference per hook operation would fail on the first assertion and a
    /// leaked reference would fail on teardown.
    #[test]
    fn every_live_connection_owns_exactly_one_protocol_reference(connections in 1_usize..=8) {
        let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
        let protocol = shared(&fixture);
        let weak = weak_handle(&protocol);
        prop_assert_eq!(strong_count(&protocol), 1, "the fixture holds one reference");

        let mut actors = Vec::with_capacity(connections);
        for live in 1..=connections {
            let actor = actor_holding(ProtocolHooks::from_protocol(&protocol))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            actors.push(actor);
            prop_assert_eq!(
                strong_count(&protocol),
                live + 1,
                "connection {} should add exactly one reference",
                live
            );
        }
        prop_assert_eq!(
            fixture.events(),
            vec![HookEvent::Setup; connections],
            "every connection dispatches setup exactly once"
        );

        let mut remaining = connections;
        while let Some(actor) = actors.pop() {
            drop(actor);
            remaining -= 1;
            prop_assert_eq!(
                strong_count(&protocol),
                remaining + 1,
                "each teardown should release exactly one reference"
            );
        }

        prop_assert!(is_alive(&weak), "the fixture keeps the protocol alive");
        prop_assert_eq!(drop_count(&fixture), 0, "no instance is dropped yet");
        drop(protocol);
        prop_assert!(!is_alive(&weak), "no strong reference remains");
        prop_assert_eq!(
            drop_count(&fixture),
            1,
            "exactly one installed protocol instance is dropped"
        );
    }
}

/// Build one actor whose hooks install the supplied protocol reference.
fn actor_holding(hooks: ProtocolHooks<Vec<u8>, ()>) -> TestResult<ConnectionActor<Vec<u8>, ()>> {
    let (queues, handle) = PushQueues::<Vec<u8>>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    Ok(ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        CancellationToken::new(),
        hooks,
    ))
}
