//! Fixture world for serializer boundary behavioural tests.

use std::sync::{Arc, Mutex};

use rstest::fixture;
use wireframe::{
    app::Envelope,
    message::DeserializeContext,
    serializer::{BincodeSerializer, Serializer},
};

#[path = "../common/context_capturing_serializer.rs"]
mod context_capturing_serializer;

use context_capturing_serializer::{CapturedDeserializeContext, ContextCapturingSerializer};
/// Shared result type used by serializer boundary fixtures and steps.
pub use wireframe_testing::TestResult;

#[derive(bincode::Decode, bincode::Encode, Debug, PartialEq, Eq)]
struct LegacyPayload {
    value: u32,
}

/// Behavioural test world for serializer boundary scenarios.
#[derive(Default)]
pub struct SerializerBoundariesWorld {
    legacy_value: Option<u32>,
    decoded_legacy_value: Option<u32>,
    context_message_id: Option<u32>,
    context_correlation_id: Option<u64>,
    captured_context: Arc<Mutex<Option<CapturedDeserializeContext>>>,
}

/// Fixture world for serializer boundary tests.
#[fixture]
pub fn serializer_boundaries_world() -> SerializerBoundariesWorld {
    SerializerBoundariesWorld::default()
}

impl SerializerBoundariesWorld {
    /// Set the legacy value used for round-trip tests.
    pub fn set_legacy_value(&mut self, value: u32) { self.legacy_value = Some(value); }

    /// # Errors
    ///
    /// Returns an error if a legacy input value was not set.
    pub fn round_trip_legacy_payload(&mut self) -> TestResult {
        let value = self.legacy_value.ok_or("legacy value not set")?;
        let serializer = BincodeSerializer;
        let payload = LegacyPayload { value };
        let bytes = serializer.serialize(&payload)?;
        let (decoded, _) = serializer.deserialize::<LegacyPayload>(&bytes)?;
        self.decoded_legacy_value = Some(decoded.value);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if no decoded legacy value has been produced.
    pub fn assert_decoded_legacy_value(&self, expected: u32) -> TestResult {
        let actual = self
            .decoded_legacy_value
            .ok_or("decoded legacy value not set")?;
        if actual != expected {
            return Err(format!("expected decoded legacy value {expected}, got {actual}").into());
        }
        Ok(())
    }

    /// Set message and correlation ids used during deserialization.
    pub fn set_deserialize_context(&mut self, message_id: u32, correlation_id: u64) {
        self.context_message_id = Some(message_id);
        self.context_correlation_id = Some(correlation_id);
    }

    /// # Errors
    ///
    /// Returns an error if context values have not been set.
    pub fn decode_with_context(&mut self) -> TestResult {
        let message_id = self
            .context_message_id
            .ok_or("context message id not set")?;
        let correlation_id = self
            .context_correlation_id
            .ok_or("context correlation id not set")?;
        let envelope = Envelope::new(message_id, Some(correlation_id), vec![1, 2, 3]);
        let encoded = BincodeSerializer.serialize(&envelope)?;
        let serializer = ContextCapturingSerializer::new(self.captured_context.clone());

        let context = DeserializeContext {
            frame_metadata: Some(&encoded),
            message_id: Some(message_id),
            correlation_id: Some(correlation_id),
            metadata_bytes_consumed: Some(encoded.len()),
        };
        let _: (Envelope, usize) = serializer.deserialize_with_context(&encoded, &context)?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if no context was captured.
    fn assert_captured_field<T>(
        &self,
        field_extractor: impl FnOnce(&CapturedDeserializeContext) -> Option<T>,
        expected: T,
        field_name: &str,
    ) -> TestResult
    where
        T: PartialEq + std::fmt::Debug,
    {
        let captured = (*self
            .captured_context
            .lock()
            .map_err(|_| "captured context mutex poisoned")?)
        .ok_or("captured context not available")?;
        let actual = field_extractor(&captured);
        let expected_value = Some(expected);
        if actual != expected_value {
            return Err(format!(
                "expected captured {field_name} {expected_value:?}, got {actual:?}"
            )
            .into());
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if no context was captured.
    pub fn assert_captured_message_id(&self, expected: u32) -> TestResult {
        self.assert_captured_field(|ctx| ctx.message_id, expected, "message id")
    }

    /// # Errors
    ///
    /// Returns an error if no context was captured.
    pub fn assert_captured_correlation_id(&self, expected: u64) -> TestResult {
        self.assert_captured_field(|ctx| ctx.correlation_id, expected, "correlation id")
    }
}
