//! Tests for push-queue and session-registry builder configuration.

use std::sync::Arc;

use bytes::Bytes;

use super::super::WireframeApp;
use crate::{
    app::{PrepareError, PreparedApp},
    codec::LengthDelimitedFrameCodec,
    push::PushQueues,
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

/// A zero-capacity template is rejected while preparing, before any
/// connection can fail on it.
#[tokio::test]
async fn prepare_rejects_invalid_push_template() {
    let template = PushQueues::<Bytes>::builder().high_capacity(0);
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
