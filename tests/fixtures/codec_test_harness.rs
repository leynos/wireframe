//! Test world for codec-aware test harness behavioural scenarios.
//!
//! Exercises the `wireframe_testing` codec-aware driver API with
//! `HotlineFrameCodec` to verify transparent encoding and decoding.

use std::sync::Arc;

use futures::future::BoxFuture;
use rstest::fixture;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::{HotlineFrame, HotlineFrameCodec},
    serializer::{BincodeSerializer, Serializer},
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// BDD world holding the app, codec, and collected responses.
///
/// `WireframeApp` does not implement `Debug`, so this type provides a manual
/// implementation that redacts the app field.
#[derive(Default)]
pub struct CodecTestHarnessWorld {
    codec: Option<HotlineFrameCodec>,
    app: Option<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>>,
    response_payloads: Vec<Vec<u8>>,
    response_frames: Vec<HotlineFrame>,
}

impl std::fmt::Debug for CodecTestHarnessWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodecTestHarnessWorld")
            .field("codec", &self.codec)
            .field("app", &self.app.as_ref().map(|_| ".."))
            .field("response_payloads", &self.response_payloads.len())
            .field("response_frames", &self.response_frames.len())
            .finish()
    }
}

/// Fixture for codec test harness scenarios used by rstest-bdd steps.
///
/// Note: rustfmt collapses simple fixtures into one line, which triggers
/// `unused_braces`, so keep `rustfmt::skip`.
#[rustfmt::skip]
#[fixture]
pub fn codec_test_harness_world() -> CodecTestHarnessWorld {
    CodecTestHarnessWorld::default()
}

impl CodecTestHarnessWorld {
    /// Configure the app with a `HotlineFrameCodec`.
    ///
    /// # Errors
    /// Returns an error if the app or route registration fails.
    pub fn configure_app(&mut self, max_frame_length: usize) -> TestResult {
        let codec = HotlineFrameCodec::new(max_frame_length);
        let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
            .with_codec(codec.clone())
            .route(
                1,
                Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) }),
            )?;
        self.codec = Some(codec);
        self.app = Some(app);
        Ok(())
    }

    /// Drive the app through the payload-level codec driver.
    ///
    /// # Errors
    /// Returns an error if serialization or the driver fails.
    pub async fn drive_payload(&mut self) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;

        let env = Envelope::new(1, Some(7), b"bdd-test".to_vec());
        let serialized = BincodeSerializer.serialize(&env)?;

        self.response_payloads =
            wireframe_testing::drive_with_codec_payloads(app, codec, vec![serialized]).await?;
        Ok(())
    }

    /// Drive the app through the frame-level codec driver.
    ///
    /// # Errors
    /// Returns an error if serialization or the driver fails.
    pub async fn drive_frames(&mut self) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;

        let env = Envelope::new(1, Some(7), b"bdd-frame-test".to_vec());
        let serialized = BincodeSerializer.serialize(&env)?;

        self.response_frames =
            wireframe_testing::drive_with_codec_frames(app, codec, vec![serialized]).await?;
        Ok(())
    }

    /// Verify that payload responses are non-empty.
    ///
    /// # Errors
    /// Returns an error if the assertion fails.
    pub fn verify_payloads_non_empty(&self) -> TestResult {
        if self.response_payloads.is_empty() {
            return Err("expected non-empty response payloads".into());
        }
        Ok(())
    }

    /// Verify that response frames contain valid transaction identifiers.
    ///
    /// # Errors
    /// Returns an error if no frames were received.
    pub fn verify_frames_have_transaction_ids(&self) -> TestResult {
        if self.response_frames.is_empty() {
            return Err("expected at least one response frame".into());
        }
        // wrap_payload assigns transaction_id = 0; confirming the field is
        // present in the decoded frame proves codec metadata survives the
        // driver pipeline.
        for frame in &self.response_frames {
            if frame.transaction_id != 0 {
                return Err(
                    format!("expected transaction_id 0, got {}", frame.transaction_id).into(),
                );
            }
        }
        Ok(())
    }
}
