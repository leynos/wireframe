//! Strong-ownership tests for installed protocols.
//!
//! A protocol-backed connection runtime must own exactly one strong reference
//! to the installed protocol, regardless of how many hook operations dispatch
//! to it. These tests read `Arc::strong_count` at each ownership boundary and
//! use a drop counter plus weak handles to prove the connection releases its
//! reference at teardown.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/recording_protocol.rs"]
mod recording_protocol;

use std::sync::Arc;

use bytes::Bytes;
use recording_protocol::{
    HookEvent,
    RecordingProtocolFixture,
    is_alive,
    strong_count,
    weak_handle,
};
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::Envelope,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::ProtocolHooks,
    push::PushQueues,
};
use wireframe_testing::TestResult;

/// Application type using the default length-delimited codec (`Bytes` frames).
type ResponseApp =
    wireframe::app::WireframeApp<wireframe::serializer::BincodeSerializer, (), Envelope>;

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

#[rstest]
fn actor_holds_exactly_one_protocol_reference() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = fixture.shared();
    assert_eq!(
        strong_count(&protocol),
        1,
        "the fixture holds one reference"
    );

    let weak = weak_handle(&protocol);
    let hooks = ProtocolHooks::from_protocol(&protocol);
    assert_eq!(
        strong_count(&protocol),
        2,
        "one hooks value owns exactly one reference, not one per operation"
    );

    let actor = actor_holding(hooks).expect("build actor over the hooks");
    assert_eq!(
        strong_count(&protocol),
        2,
        "the actor takes ownership of that single reference"
    );
    assert_eq!(
        fixture.events(),
        vec![HookEvent::Setup],
        "the actor ran setup once through its single hook owner"
    );

    drop(actor);
    assert_eq!(
        strong_count(&protocol),
        1,
        "the connection releases its reference at teardown"
    );
    assert!(
        is_alive(&weak),
        "the fixture reference keeps the protocol alive"
    );
    assert_eq!(
        fixture.drops(),
        0,
        "the protocol is not dropped while a reference remains"
    );

    drop(protocol);
    assert!(!is_alive(&weak), "no strong reference remains");
    assert_eq!(
        fixture.drops(),
        1,
        "exactly one installed protocol instance was dropped"
    );
}

#[rstest]
fn barrier_held_connections_each_add_one_owner() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = fixture.shared();

    let mut actors = Vec::new();
    for _ in 0..3 {
        actors.push(actor_holding(ProtocolHooks::from_protocol(&protocol)).expect("build actor"));
    }
    assert_eq!(
        strong_count(&protocol),
        4,
        "three concurrently open connections hold one reference each"
    );
    assert_eq!(
        fixture.events(),
        vec![HookEvent::Setup; 3],
        "each connection ran setup exactly once"
    );

    while let Some(actor) = actors.pop() {
        drop(actor);
    }
    assert_eq!(
        strong_count(&protocol),
        1,
        "each connection releases exactly one reference"
    );
}

#[rstest]
fn each_hooks_value_owns_one_reference() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = fixture.shared();

    let hooks: Vec<ProtocolHooks<Vec<u8>, ()>> = (0..3)
        .map(|_| ProtocolHooks::from_protocol(&protocol))
        .collect();
    assert_eq!(
        strong_count(&protocol),
        4,
        "every hooks value adds exactly one reference"
    );

    drop(hooks);
    assert_eq!(
        strong_count(&protocol),
        1,
        "all hooks released their references"
    );
}

#[rstest]
fn app_accessor_returns_the_single_installed_protocol() {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let app = ResponseApp::new()
        .expect("build application")
        .with_protocol(fixture.protocol());

    let handle = app.protocol().expect("protocol should be installed");
    let weak = Arc::downgrade(&handle);
    assert_eq!(
        Arc::strong_count(&handle),
        2,
        "the app holds one reference and the accessor returned its clone"
    );

    let again = app.protocol().expect("protocol should remain installed");
    assert!(
        Arc::ptr_eq(&handle, &again),
        "the accessor must return the installed protocol, not a new value"
    );
    assert_eq!(
        Arc::strong_count(&handle),
        3,
        "each accessor clone is one additional reference"
    );

    let hooks = app.protocol_hooks();
    assert_eq!(
        Arc::strong_count(&handle),
        4,
        "one connection's hooks add exactly one reference"
    );

    drop(hooks);
    drop(again);
    assert_eq!(
        Arc::strong_count(&handle),
        2,
        "dropping the hooks leaves only the app and this probe"
    );

    drop(handle);
    assert!(is_alive(&weak), "the app still owns the protocol");
    let still_installed = app.protocol().expect("protocol should still be installed");
    assert_eq!(
        Arc::strong_count(&still_installed),
        2,
        "the app retains exactly one reference between connections"
    );
    drop(still_installed);

    drop(app);
    assert!(!is_alive(&weak), "app teardown releases its reference");
    assert_eq!(
        fixture.drops(),
        1,
        "exactly one installed protocol instance was dropped"
    );
}
