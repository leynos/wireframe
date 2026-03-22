//! BDD world fixture for `wireframe::testkit` export scenarios.

use std::{io, num::NonZeroUsize, sync::Arc};

use futures::future::BoxFuture;
use rstest::fixture;
pub use wireframe::testkit::TestResult;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    message_assembler::{
        AssembledMessage,
        EnvelopeId,
        EnvelopeRouting,
        MessageAssemblyError,
        MessageKey,
    },
    serializer::{BincodeSerializer, Serializer},
    testkit::{
        MessageAssemblySnapshot,
        assert_message_assembly_completed_for_key,
        drive_with_partial_frames,
    },
};

#[derive(Default)]
pub struct TestkitExportWorld {
    codec: Option<HotlineFrameCodec>,
    app: Option<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>>,
    response_payloads: Vec<Vec<u8>>,
    last_result: Option<Result<Option<AssembledMessage>, MessageAssemblyError>>,
    completed_messages: Vec<AssembledMessage>,
    reassembly_assertion_passed: bool,
}

impl std::fmt::Debug for TestkitExportWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestkitExportWorld")
            .field("codec", &self.codec)
            .field("app", &self.app.as_ref().map(|_| ".."))
            .field("response_payloads", &self.response_payloads.len())
            .field("has_snapshot", &self.last_result.is_some())
            .field("completed_messages", &self.completed_messages.len())
            .field(
                "reassembly_assertion_passed",
                &self.reassembly_assertion_passed,
            )
            .finish()
    }
}

#[rustfmt::skip]
#[fixture]
pub fn testkit_export_world() -> TestkitExportWorld {
    TestkitExportWorld::default()
}

impl TestkitExportWorld {
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

    pub async fn drive_chunked(&mut self, chunk_size: usize) -> TestResult {
        let app = self.app.take().ok_or("app not configured")?;
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        let chunk = NonZeroUsize::new(chunk_size).ok_or("chunk size must be non-zero")?;
        let payload =
            BincodeSerializer.serialize(&Envelope::new(1, Some(7), b"bdd-testkit".to_vec()))?;
        self.response_payloads =
            drive_with_partial_frames(app, codec, vec![payload], chunk).await?;
        Ok(())
    }

    pub fn seed_completed_message_snapshot(&mut self, key: u64) {
        let routing = EnvelopeRouting {
            envelope_id: EnvelopeId(1),
            correlation_id: None,
        };
        let assembled = AssembledMessage::new(MessageKey(key), routing, vec![], b"done".to_vec());
        self.last_result = Some(Ok(Some(assembled.clone())));
        self.completed_messages = vec![assembled];
    }

    pub fn assert_root_reassembly_helper(&mut self) -> TestResult {
        let snapshot = MessageAssemblySnapshot::new(
            self.last_result.as_ref(),
            &self.completed_messages,
            &[],
            0,
            0,
        );
        assert_message_assembly_completed_for_key(snapshot, MessageKey(7), b"done")?;
        self.reassembly_assertion_passed = true;
        Ok(())
    }

    pub fn assert_response_payloads_non_empty(&self) -> TestResult {
        if self.response_payloads.is_empty() {
            return Err("expected non-empty response payloads".into());
        }
        let first_payload = self
            .response_payloads
            .first()
            .ok_or("expected non-empty response payloads")?;
        let (envelope, consumed) = BincodeSerializer
            .deserialize::<Envelope>(first_payload)
            .map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidData, format!("deserialize: {error}"))
            })?;
        if consumed != first_payload.len() {
            return Err(format!(
                "deserialize: trailing bytes after envelope: consumed {consumed} of {}",
                first_payload.len()
            )
            .into());
        }
        if envelope.payload_bytes() != b"bdd-testkit" {
            return Err(format!("unexpected payload bytes: {:?}", envelope.payload_bytes()).into());
        }
        Ok(())
    }

    pub fn assert_reassembly_assertion_passed(&self) -> TestResult {
        if self.reassembly_assertion_passed {
            Ok(())
        } else {
            Err("expected reassembly assertion to pass".into())
        }
    }
}
