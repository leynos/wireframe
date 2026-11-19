//! Strong-ownership tests for installed protocols.
//!
//! A protocol-backed connection runtime must own exactly one strong reference
//! to the installed protocol, regardless of how many hook operations dispatch
//! to it. These tests read `Arc::strong_count` at each ownership boundary and
//! use a drop counter plus weak handles to prove the connection releases its
//! reference at teardown.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/protocol_request.rs"]
mod protocol_request;
#[path = "common/recording_protocol.rs"]
mod recording_protocol;
#[path = "common/recording_protocol_ownership.rs"]
mod recording_protocol_ownership;

use std::sync::{Arc, Mutex, Weak};

use bytes::Bytes;
use protocol_request::request_frame;
use recording_protocol::{HookEvent, RecordingProtocolFixture};
use recording_protocol_ownership::{drop_count, is_alive, shared, strong_count, weak_handle};
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Handler},
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ProtocolHooks, WireframeProtocol},
    push::PushQueues,
};
use wireframe_testing::{TestResult, drive_prepared_with_frames};

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

/// Type of the protocol reference a connection installs.
type InstalledProtocol = Arc<dyn WireframeProtocol<Frame = Bytes, ProtocolError = ()>>;

/// Published view of the installed protocol the handler observes.
type ProbeSlot = Arc<Mutex<Option<Weak<dyn WireframeProtocol<Frame = Bytes, ProtocolError = ()>>>>>;

/// Slot the test publishes the probe into once preparation has run.
type CountLog = Arc<Mutex<Vec<usize>>>;

/// Handler that records the protocol's strong count while the connection runs.
///
/// The handler executes inside the live connection, so a count read here sees
/// the connection's own reference. A weak handle keeps the handler from
/// changing the count it reports.
fn counting_handler(probe: ProbeSlot, counts: CountLog) -> Handler<Envelope> {
    Arc::new(move |_env: &Envelope| {
        if let Some(weak) = probe.lock().expect("probe slot").as_ref() {
            counts.lock().expect("count log").push(weak.strong_count());
        }
        Box::pin(async {})
    })
}

/// A prepared connection holds exactly one strong protocol reference.
///
/// The prepared template owns one, the live connection adds exactly one while
/// it serves a request, and teardown returns the count to the template's.
#[rstest]
#[tokio::test]
async fn prepared_connection_holds_exactly_one_protocol_reference() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let probe_slot: ProbeSlot = Arc::default();
    let counts: CountLog = Arc::default();
    let app = ResponseApp::new()?
        .route(
            1,
            counting_handler(Arc::clone(&probe_slot), Arc::clone(&counts)),
        )?
        .with_protocol(fixture.protocol());
    let prepared = app
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let probe: InstalledProtocol = prepared.protocol().expect("protocol should be installed");
    let weak = Arc::downgrade(&probe);
    assert_eq!(
        Arc::strong_count(&probe),
        2,
        "the prepared template holds one reference and the accessor returned its clone"
    );
    *probe_slot.lock().expect("probe slot") = Some(Arc::downgrade(&probe));

    drive_prepared_with_frames(&prepared, vec![request_frame(1)?]).await?;

    assert_eq!(
        *counts.lock().expect("count log"),
        vec![3],
        "the live connection added exactly one reference to the template's"
    );
    assert_eq!(
        Arc::strong_count(&probe),
        2,
        "the finished connection released its single reference"
    );
    assert!(
        fixture
            .events()
            .iter()
            .any(|event| matches!(event, HookEvent::CommandEnd)),
        "the connection's own hook owner ran the completion hook: {:?}",
        fixture.events()
    );

    drop(probe);
    assert!(
        is_alive(&weak),
        "the prepared template still owns the protocol"
    );
    drop(prepared);
    assert!(!is_alive(&weak), "preparing released no other reference");
    assert_eq!(
        drop_count(&fixture),
        1,
        "exactly one installed protocol instance was dropped"
    );
    Ok(())
}

#[rstest]
fn actor_holds_exactly_one_protocol_reference() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = shared(&fixture);
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
        drop_count(&fixture),
        0,
        "the protocol is not dropped while a reference remains"
    );

    drop(protocol);
    assert!(!is_alive(&weak), "no strong reference remains");
    assert_eq!(
        drop_count(&fixture),
        1,
        "exactly one installed protocol instance was dropped"
    );
}

#[rstest]
fn barrier_held_connections_each_add_one_owner() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = shared(&fixture);

    let mut actors = Vec::new();
    let mut built_counts = Vec::new();
    for _ in 0..3 {
        actors.push(actor_holding(ProtocolHooks::from_protocol(&protocol)).expect("build actor"));
        built_counts.push(strong_count(&protocol));
    }
    assert_eq!(
        built_counts,
        vec![2, 3, 4],
        "each concurrently open connection adds exactly one reference"
    );
    assert_eq!(
        fixture.events(),
        vec![HookEvent::Setup; 3],
        "each connection ran setup exactly once"
    );

    let mut released_counts = Vec::new();
    while let Some(actor) = actors.pop() {
        drop(actor);
        released_counts.push(strong_count(&protocol));
    }
    assert_eq!(
        released_counts,
        vec![3, 2, 1],
        "each connection releases exactly one reference at teardown"
    );
}

#[rstest]
fn each_hooks_value_owns_one_reference() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = shared(&fixture);

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
        drop_count(&fixture),
        1,
        "exactly one installed protocol instance was dropped"
    );
}
