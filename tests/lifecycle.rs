#![cfg(not(loom))]
//! Tests for connection lifecycle callbacks.
//!
//! They check setup, teardown, and state propagation through helper utilities.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::BytesMut;
use tokio_util::codec::Encoder;
use wireframe::{
    app::{Envelope, Packet, PacketParts},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    TEST_MAX_FRAME,
    decode_frames,
    new_test_codec,
    run_app,
    run_with_duplex_server,
};

mod common;
use common::TestResult;

type App<E> = wireframe::app::WireframeApp<BincodeSerializer, u32, E>;
type BasicApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

fn call_counting_callback<R, A>(
    counter: &Arc<AtomicUsize>,
    result: R,
) -> impl Fn(A) -> Pin<Box<dyn Future<Output = R> + Send>> + Clone + 'static
where
    A: Send + 'static,
    R: Clone + Send + 'static,
{
    let counter = counter.clone();
    move |_| {
        let counter = counter.clone();
        let result = result.clone();
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            result
        })
    }
}

fn wireframe_app_with_lifecycle_callbacks<E>(
    setup: &Arc<AtomicUsize>,
    teardown: &Arc<AtomicUsize>,
    state: u32,
) -> wireframe::app::Result<App<E>>
where
    E: Packet,
{
    let setup_cb = call_counting_callback(setup, state);
    let teardown_cb = call_counting_callback(teardown, ());

    let app = App::<E>::new()?
        .on_connection_setup(move || setup_cb(()))?
        .on_connection_teardown(teardown_cb)?;

    Ok(app)
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn setup_and_teardown_callbacks_run() -> TestResult<()> {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));

    let app =
        wireframe_app_with_lifecycle_callbacks::<Envelope>(&setup_count, &teardown_count, 42)?;

    run_with_duplex_server(app).await;

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback did not run exactly once"
    );
    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback did not run exactly once"
    );

    Ok(())
}
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn setup_without_teardown_runs() -> TestResult<()> {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let cb = call_counting_callback(&setup_count, ());

    let app = BasicApp::new()?.on_connection_setup(move || cb(()))?;

    run_with_duplex_server(app).await;

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback did not run"
    );

    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn teardown_without_setup_does_not_run() -> TestResult<()> {
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let cb = call_counting_callback(&teardown_count, ());

    let app = BasicApp::new()?.on_connection_teardown(cb)?;

    run_with_duplex_server(app).await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        0,
        "teardown callback should not run"
    );

    Ok(())
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct StateEnvelope {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

impl Packet for StateEnvelope {
    fn id(&self) -> u32 { self.id }

    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        let id = parts.id();
        let correlation_id = parts.correlation_id();
        let payload = parts.payload();
        Self {
            id,
            correlation_id,
            payload,
        }
    }
}

#[tokio::test]
async fn helpers_preserve_correlation_id_and_run_callbacks() -> TestResult<()> {
    let setup = Arc::new(AtomicUsize::new(0));
    let teardown = Arc::new(AtomicUsize::new(0));

    let app = wireframe_app_with_lifecycle_callbacks::<StateEnvelope>(&setup, &teardown, 7)?
        .route(1, Arc::new(|_: &StateEnvelope| Box::pin(async {})))?;

    let env = StateEnvelope {
        id: 1,
        correlation_id: Some(0),
        payload: vec![1],
    };
    let bytes = BincodeSerializer.serialize(&env)?;
    let mut frame = BytesMut::with_capacity(bytes.len() + 4);
    let mut codec = new_test_codec(TEST_MAX_FRAME);
    codec.encode(bytes.into(), &mut frame)?;

    let out = run_app(app, vec![frame.to_vec()], None).await?;
    if out.is_empty() {
        return Err("expected response frames".into());
    }

    let frames = decode_frames(out);
    if frames.len() != 1 {
        return Err("expected a single response frame".into());
    }
    let first = frames.first().ok_or("response frames unexpectedly empty")?;
    let (resp, _) = BincodeSerializer.deserialize::<StateEnvelope>(first)?;
    if resp.correlation_id != Some(0) {
        return Err("correlation id not preserved".into());
    }

    if setup.load(Ordering::SeqCst) != 1 {
        return Err("setup callback did not run exactly once".into());
    }
    if teardown.load(Ordering::SeqCst) != 1 {
        return Err("teardown callback did not run exactly once".into());
    }

    Ok(())
}
