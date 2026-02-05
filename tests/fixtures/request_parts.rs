//! `RequestPartsWorld` fixture for rstest-bdd tests.
//!
//! Converted from Cucumber World to rstest fixture. The struct and its methods
//! remain unchanged; only the trait derivation and fixture function are added.

use std::str::FromStr;

use rstest::fixture;
use wireframe::request::RequestParts;
/// Re-export `TestResult` from common for use in steps.
pub use wireframe_testing::TestResult;

/// Request identifier wrapper for test steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestId(pub u32);

impl FromStr for RequestId {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(RequestId) }
}

/// Correlation identifier wrapper for test steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelationId(pub u64);

impl FromStr for CorrelationId {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(CorrelationId) }
}

/// Metadata byte wrapper for test steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataByte(pub u8);

impl FromStr for MetadataByte {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(MetadataByte) }
}

/// Test world exercising `RequestParts` metadata handling.
#[derive(Debug, Default)]
pub struct RequestPartsWorld {
    parts: Option<RequestParts>,
}

/// Fixture for `RequestPartsWorld`.
#[rustfmt::skip]
#[fixture]
pub fn request_parts_world() -> RequestPartsWorld {
    RequestPartsWorld::default()
}

impl RequestPartsWorld {
    /// Create request parts with all fields specified.
    pub fn create_parts(
        &mut self,
        id: RequestId,
        correlation_id: Option<CorrelationId>,
        metadata: Vec<MetadataByte>,
    ) {
        self.parts = Some(RequestParts::new(
            id.0,
            correlation_id.map(|value| value.0),
            metadata.into_iter().map(|value| value.0).collect(),
        ));
    }

    /// Inherit a correlation id from an external source.
    ///
    /// # Errors
    /// Returns an error if parts have not been created.
    pub fn inherit_correlation(&mut self, source: Option<CorrelationId>) -> TestResult {
        let parts = self.parts.take().ok_or("request parts not created")?;
        self.parts = Some(parts.inherit_correlation(source.map(|value| value.0)));
        Ok(())
    }

    /// Append a byte to the metadata.
    ///
    /// # Errors
    /// Returns an error if parts have not been created.
    pub fn append_metadata_byte(&mut self, byte: MetadataByte) -> TestResult {
        let parts = self.parts.as_mut().ok_or("request parts not created")?;
        parts.metadata_mut().push(byte.0);
        Ok(())
    }

    /// Assert the request id matches the expected value.
    ///
    /// # Errors
    /// Returns an error if parts are missing or id does not match.
    pub fn assert_id(&self, expected: RequestId) -> TestResult {
        let parts = self.parts.as_ref().ok_or("request parts not created")?;
        if parts.id() != expected.0 {
            return Err(format!("expected id {}, got {}", expected.0, parts.id()).into());
        }
        Ok(())
    }

    /// Assert the correlation id matches the expected value.
    ///
    /// # Errors
    /// Returns an error if parts are missing or correlation id does not match.
    pub fn assert_correlation_id(&self, expected: Option<CorrelationId>) -> TestResult {
        let parts = self.parts.as_ref().ok_or("request parts not created")?;
        let expected_raw = expected.map(|value| value.0);
        if parts.correlation_id() != expected_raw {
            return Err(format!(
                "expected correlation_id {:?}, got {:?}",
                expected_raw,
                parts.correlation_id()
            )
            .into());
        }
        Ok(())
    }

    /// Assert the metadata length matches the expected value.
    ///
    /// # Errors
    /// Returns an error if parts are missing or length does not match.
    pub fn assert_metadata_length(&self, expected: usize) -> TestResult {
        let parts = self.parts.as_ref().ok_or("request parts not created")?;
        if parts.metadata().len() != expected {
            return Err(format!(
                "expected metadata length {expected}, got {}",
                parts.metadata().len()
            )
            .into());
        }
        Ok(())
    }
}
