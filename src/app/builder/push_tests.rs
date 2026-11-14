//! Tests for push-queue and session-registry builder configuration.

use std::sync::Arc;

use bytes::Bytes;
use rstest::rstest;

use super::super::WireframeApp;
use crate::{
    app::{PrepareError, PreparedApp},
    codec::LengthDelimitedFrameCodec,
    push::{PushError, PushQueues, PushQueuesBuilder},
    session::{ConnectionId, SessionRegistry},
};

/// A supplied registry is the one connections and callers both observe.
#[test]
fn supplied_session_registry_is_shared() {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let app: WireframeApp = WireframeApp::new()
        .expect("builder should initialize")
        .with_session_registry(Arc::clone(&registry));

    assert!(
        Arc::ptr_eq(&app.session_registry(), &registry),
        "builder should expose the supplied registry"
    );
}

/// A builder without an explicit registry still owns one, starting at the
/// first identifier.
#[test]
fn default_session_registry_allocates_identifiers() {
    let app: WireframeApp = WireframeApp::new().expect("builder should initialize");

    assert_eq!(
        app.session_registry().next_connection_id(),
        ConnectionId::new(1),
        "a fresh registry should start at one"
    );
}

/// Replacing the codec changes the frame type, so the push template and the
/// registry keyed by the old frame type must not survive.
#[test]
fn with_codec_resets_push_configuration() {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let template = PushQueues::<Bytes>::builder()
        .high_capacity(8)
        .low_capacity(8);
    let builder: WireframeApp = WireframeApp::default();
    let app = builder
        .with_push_queues(template)
        .with_session_registry(Arc::clone(&registry))
        .with_codec(LengthDelimitedFrameCodec::new(1024));

    assert!(
        app.push_queues.is_none(),
        "with_codec should drop the frame-typed template"
    );
    assert!(
        !Arc::ptr_eq(&app.session_registry(), &registry),
        "with_codec should replace the frame-typed registry"
    );
}

/// A template that cannot back a Tokio channel is rejected while preparing,
/// before any connection can fail on it.
#[rstest]
#[case::zero_capacity(PushQueues::<Bytes>::builder().high_capacity(0))]
#[case::oversize_capacity(PushQueues::<Bytes>::builder().high_capacity(usize::MAX))]
#[tokio::test]
async fn prepare_rejects_invalid_push_template(#[case] template: PushQueuesBuilder<Bytes>) {
    let app: WireframeApp = WireframeApp::new()
        .expect("builder should initialize")
        .with_push_queues(template);

    let result = app.prepare().await;

    assert!(
        matches!(result, Err(PrepareError::InvalidPushConfig { .. })),
        "preparation should reject an unusable template"
    );
}

/// A prepared application always offers a usable template, defaulting to the
/// standard queue configuration.
#[tokio::test]
async fn prepared_app_exposes_default_push_template() -> Result<(), Box<dyn std::error::Error>> {
    let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;

    let (_queues, _handle) = prepared.push_queues().build()?;
    Ok(())
}

/// The prepared application hands out the same registry the builder held.
#[tokio::test]
async fn prepared_app_retains_session_registry() -> Result<(), Box<dyn std::error::Error>> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let prepared: PreparedApp = WireframeApp::new()?
        .with_session_registry(Arc::clone(&registry))
        .prepare()
        .await?;

    if !Arc::ptr_eq(&prepared.session_registry(), &registry) {
        return Err("preparation should preserve the supplied registry".into());
    }
    Ok(())
}

/// The template handed to each connection rejects a push the codec cannot
/// encode, and accepts one at the limit.
#[tokio::test]
async fn prepared_push_template_enforces_the_codec_limit() -> Result<(), Box<dyn std::error::Error>>
{
    let builder: WireframeApp = WireframeApp::new()?;
    let prepared: PreparedApp = builder
        .with_codec(LengthDelimitedFrameCodec::new(64))
        .prepare()
        .await?;
    let (_queues, handle) = prepared.push_queues().build()?;

    let oversized = handle
        .push_high_priority(Bytes::from_static(&[0; 65]))
        .await;
    if oversized != Err(PushError::FrameTooLarge { size: 65, max: 64 }) {
        return Err("the connection template must carry the codec limit".into());
    }
    if handle
        .push_high_priority(Bytes::from_static(&[0; 64]))
        .await
        .is_err()
    {
        return Err("a frame at the codec limit must be accepted".into());
    }
    Ok(())
}

/// A template limit stricter than the codec's survives preparation.
///
/// Applying the codec ceiling must never widen a bound the application chose,
/// because the connection handle is the only place that bound is enforced.
#[tokio::test]
async fn prepared_push_template_keeps_a_stricter_limit() -> Result<(), Box<dyn std::error::Error>> {
    let template = PushQueues::<Bytes>::builder()
        .high_capacity(1)
        .max_frame_len(64, |frame: &Bytes| frame.len());
    let builder: WireframeApp = WireframeApp::new()?;
    let prepared: PreparedApp = builder
        .with_codec(LengthDelimitedFrameCodec::new(2048))
        .with_push_queues(template)
        .prepare()
        .await?;
    let (_queues, handle) = prepared.push_queues().build()?;

    let oversized = handle
        .push_high_priority(Bytes::from_static(&[0; 65]))
        .await;
    if oversized != Err(PushError::FrameTooLarge { size: 65, max: 64 }) {
        return Err("the stricter template limit must survive the codec ceiling".into());
    }
    Ok(())
}
