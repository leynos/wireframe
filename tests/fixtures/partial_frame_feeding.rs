//! BDD world fixture for partial frame and fragment feeding scenarios.
//!
//! Tracks application configuration, fragmenter state, and collected
//! responses across behavioural test steps.

use std::{num::NonZeroUsize, sync::Arc};

use futures::future::BoxFuture;
use rstest::fixture;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    fragment::Fragmenter,
    serializer::{BincodeSerializer, Serializer},
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// BDD world holding the app, codec, fragmenter, and collected responses.
///
/// `WireframeApp` does not implement `Debug`, so this type provides a manual
/// implementation that redacts the app field.
#[derive(Default)]
pub struct PartialFrameFeedingWorld {
    codec: Option<HotlineFrameCodec>,
    app: Option<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>>,
    fragmenter: Option<Fragmenter>,
    response_payloads: Vec<Vec<u8>>,
    fragment_feeding_completed: bool,
}

impl std::fmt::Debug for PartialFrameFeedingWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartialFrameFeedingWorld")
            .field("codec", &self.codec)
            .field("app", &self.app.as_ref().map(|_| ".."))
            .field("fragmenter", &self.fragmenter)
            .field("response_payloads", &self.response_payloads.len())
            .field(
                "fragment_feeding_completed",
                &self.fragment_feeding_completed,
            )
            .finish()
    }
}

/// Fixture for partial frame feeding scenarios used by rstest-bdd steps.
///
/// Note: rustfmt collapses simple fixtures into one line, which triggers
/// `unused_braces`, so keep `rustfmt::skip`.
#[rustfmt::skip]
#[fixture]
pub fn partial_frame_feeding_world() -> PartialFrameFeedingWorld {
    PartialFrameFeedingWorld::default()
}

impl PartialFrameFeedingWorld {
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

    /// Configure a fragmenter with the given maximum payload per fragment.
    ///
    /// # Errors
    /// Returns an error if the payload cap is zero.
    pub fn configure_fragmenter(&mut self, max_payload: usize) -> TestResult {
        let cap = NonZeroUsize::new(max_payload).ok_or("fragment payload cap must be non-zero")?;
        self.fragmenter = Some(Fragmenter::new(cap));
        Ok(())
    }

    /// Drive the app with a single payload chunked into `chunk_size` pieces.
    ///
    /// # Errors
    /// Returns an error if the app or codec is not configured, or driving fails.
    pub async fn drive_chunked(&mut self, chunk_size: usize) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        let chunk = NonZeroUsize::new(chunk_size).ok_or("chunk size must be non-zero")?;

        let env = Envelope::new(1, Some(7), b"bdd-chunked".to_vec());
        let serialized = BincodeSerializer.serialize(&env)?;

        self.response_payloads =
            wireframe_testing::drive_with_partial_frames(app, codec, vec![serialized], chunk)
                .await?;
        Ok(())
    }

    /// Drive the app with `count` payloads chunked into `chunk_size` pieces.
    ///
    /// # Errors
    /// Returns an error if the app or codec is not configured, or driving fails.
    pub async fn drive_chunked_multiple(&mut self, count: usize, chunk_size: usize) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        let chunk = NonZeroUsize::new(chunk_size).ok_or("chunk size must be non-zero")?;

        let mut payloads = Vec::with_capacity(count);
        for i in 0..count {
            let byte = u8::try_from(i)
                .map_err(|_| format!("payload index {i} exceeds u8 range; use count <= 256"))?;
            let env = Envelope::new(1, Some(7), vec![byte]);
            payloads.push(BincodeSerializer.serialize(&env)?);
        }

        self.response_payloads =
            wireframe_testing::drive_with_partial_frames(app, codec, payloads, chunk).await?;
        Ok(())
    }

    /// Drive the app with a fragmented payload.
    ///
    /// # Errors
    /// Returns an error if the app, codec, or fragmenter is not configured.
    pub async fn drive_fragmented(&mut self, payload_len: usize) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        let fragmenter = self
            .fragmenter
            .as_ref()
            .ok_or("fragmenter not configured")?;

        let _payloads =
            wireframe_testing::drive_with_fragments(app, codec, fragmenter, vec![0; payload_len])
                .await?;
        self.fragment_feeding_completed = true;
        Ok(())
    }

    /// Drive the app with a fragmented payload fed in chunks.
    ///
    /// # Errors
    /// Returns an error if the app, codec, or fragmenter is not configured.
    pub async fn drive_partial_fragmented(
        &mut self,
        payload_len: usize,
        chunk_size: usize,
    ) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        let fragmenter = self
            .fragmenter
            .as_ref()
            .ok_or("fragmenter not configured")?;
        let chunk = NonZeroUsize::new(chunk_size).ok_or("chunk size must be non-zero")?;

        let _payloads = wireframe_testing::drive_with_partial_fragments(
            app,
            codec,
            fragmenter,
            vec![0; payload_len],
            chunk,
        )
        .await?;
        self.fragment_feeding_completed = true;
        Ok(())
    }

    /// Assert that fragment feeding completed without error.
    ///
    /// # Errors
    /// Returns an error if fragment feeding has not been attempted or failed.
    pub fn assert_fragment_feeding_completed(&self) -> TestResult {
        if !self.fragment_feeding_completed {
            return Err("fragment feeding has not completed successfully".into());
        }
        Ok(())
    }

    /// Assert that response payloads are non-empty.
    ///
    /// # Errors
    /// Returns an error if no response payloads were collected.
    pub fn assert_payloads_non_empty(&self) -> TestResult {
        if self.response_payloads.is_empty() {
            return Err("expected non-empty response payloads".into());
        }
        Ok(())
    }

    /// Assert that the response contains exactly `expected` payloads.
    ///
    /// # Errors
    /// Returns an error if the payload count does not match.
    pub fn assert_payload_count(&self, expected: usize) -> TestResult {
        let actual = self.response_payloads.len();
        if actual != expected {
            return Err(format!("expected {expected} response payloads, got {actual}").into());
        }
        Ok(())
    }
}
