//! Test world for request parts scenarios.
#![cfg(not(loom))]

use cucumber::World;
use wireframe::request::RequestParts;

use super::TestResult;

/// Test world exercising `RequestParts` metadata handling.
#[derive(Debug, Default, World)]
pub struct RequestPartsWorld {
    parts: Option<RequestParts>,
}

impl RequestPartsWorld {
    /// Create request parts with all fields specified.
    pub fn create_parts(&mut self, id: u32, correlation_id: Option<u64>, metadata: Vec<u8>) {
        self.parts = Some(RequestParts::new(id, correlation_id, metadata));
    }

    /// Inherit a correlation id from an external source.
    ///
    /// # Errors
    /// Returns an error if parts have not been created.
    pub fn inherit_correlation(&mut self, source: Option<u64>) -> TestResult {
        let parts = self.parts.take().ok_or("request parts not created")?;
        self.parts = Some(parts.inherit_correlation(source));
        Ok(())
    }

    /// Append a byte to the metadata.
    ///
    /// # Errors
    /// Returns an error if parts have not been created.
    pub fn append_metadata_byte(&mut self, byte: u8) -> TestResult {
        let parts = self.parts.as_mut().ok_or("request parts not created")?;
        parts.metadata_mut().push(byte);
        Ok(())
    }

    /// Assert the request id matches the expected value.
    ///
    /// # Errors
    /// Returns an error if parts are missing or id does not match.
    pub fn assert_id(&self, expected: u32) -> TestResult {
        let parts = self.parts.as_ref().ok_or("request parts not created")?;
        if parts.id() != expected {
            return Err(format!("expected id {expected}, got {}", parts.id()).into());
        }
        Ok(())
    }

    /// Assert the correlation id matches the expected value.
    ///
    /// # Errors
    /// Returns an error if parts are missing or correlation id does not match.
    pub fn assert_correlation_id(&self, expected: Option<u64>) -> TestResult {
        let parts = self.parts.as_ref().ok_or("request parts not created")?;
        if parts.correlation_id() != expected {
            return Err(format!(
                "expected correlation_id {:?}, got {:?}",
                expected,
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
