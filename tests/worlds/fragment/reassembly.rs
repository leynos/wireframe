//! Reassembly-focused helpers for `FragmentWorld`.

use std::{num::NonZeroUsize, time::Duration};

use super::{
    FragmentError,
    FragmentHeader,
    FragmentWorld,
    MessageId,
    Reassembler,
    ReassemblyError,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl FragmentWorld {
    /// Configure a reassembler with size and timeout guards.
    ///
    /// # Errors
    /// Returns an error when the message size is zero or the configuration
    /// cannot be constructed.
    pub fn configure_reassembler(
        &mut self,
        max_message_size: usize,
        timeout_secs: u64,
    ) -> TestResult {
        let size = NonZeroUsize::new(max_message_size).ok_or("reassembly cap must be non-zero")?;
        self.reassembler = Some(Reassembler::new(size, Duration::from_secs(timeout_secs)));
        self.last_reassembled = None;
        self.last_reassembly_error = None;
        self.last_evicted.clear();
        Ok(())
    }

    /// Submit a fragment to the configured reassembler.
    ///
    /// # Errors
    /// Returns an error if the reassembler is missing.
    pub fn push_fragment(&mut self, header: FragmentHeader, payload_len: usize) -> TestResult {
        let reassembler = self
            .reassembler
            .as_mut()
            .ok_or("reassembler not configured")?;
        let payload = vec![0_u8; payload_len];
        self.last_reassembly_error = None;
        self.last_reassembled = None;
        match reassembler.push_at(header, payload, self.now) {
            Ok(output) => self.last_reassembled = output,
            Err(err) => self.last_reassembly_error = Some(err),
        }
        Ok(())
    }

    /// Advance the simulated clock.
    ///
    /// # Errors
    /// Returns an error if the simulated clock would overflow.
    pub fn advance_time(&mut self, delta: Duration) -> TestResult {
        self.now = self
            .now
            .checked_add(delta)
            .ok_or("time advance overflowed")?;
        Ok(())
    }

    /// Purge expired partial messages based on the current clock reading.
    ///
    /// # Errors
    /// Returns an error if the reassembler has not been configured.
    pub fn purge_reassembly(&mut self) -> TestResult {
        let reassembler = self
            .reassembler
            .as_mut()
            .ok_or("reassembler not configured")?;
        self.last_evicted = reassembler.purge_expired_at(self.now);
        Ok(())
    }

    /// Assert that a message has been reassembled with the expected payload
    /// length.
    ///
    /// # Errors
    /// Returns an error if no message has been reassembled or the length does
    /// not match the expectation.
    pub fn assert_reassembled_len(&self, expected_len: usize) -> TestResult {
        let message = self
            .last_reassembled
            .as_ref()
            .ok_or("no message reassembled")?;
        if message.payload().len() != expected_len {
            return Err("payload length mismatch".into());
        }
        Ok(())
    }

    /// Assert that no message has been fully reassembled.
    ///
    /// # Errors
    /// Returns an error if a message has already been reassembled.
    pub fn assert_no_reassembly(&self) -> TestResult {
        if self.last_reassembled.is_some() {
            return Err("unexpected reassembled message present".into());
        }
        Ok(())
    }

    /// Helper for asserting on the latest captured reassembly error.
    ///
    /// # Errors
    /// Returns an error when no reassembly error was captured or the predicate
    /// does not match the error variant.
    fn assert_reassembly_error_matches<F>(
        &self,
        predicate: F,
        expected_description: &str,
    ) -> TestResult
    where
        F: FnOnce(&ReassemblyError) -> bool,
    {
        let err = self
            .last_reassembly_error
            .as_ref()
            .ok_or("no reassembly error captured")?;
        if !predicate(err) {
            return Err(format!("expected {expected_description}, got {err}").into());
        }
        Ok(())
    }

    /// Assert the latest reassembly error signalled an over-limit message.
    ///
    /// # Errors
    /// Returns an error if no reassembly error was captured or it was not a
    /// message-too-large error.
    pub fn assert_reassembly_over_limit(&self) -> TestResult {
        self.assert_reassembly_error_matches(
            |err| matches!(err, ReassemblyError::MessageTooLarge { .. }),
            "message-too-large error",
        )
    }

    /// Assert that the latest reassembly error was triggered by an out-of-order
    /// fragment.
    ///
    /// # Errors
    /// Returns an error if no reassembly error was captured or it was not an
    /// index-mismatch error.
    pub fn assert_reassembly_out_of_order(&self) -> TestResult {
        self.assert_reassembly_error_matches(
            |err| {
                matches!(
                    err,
                    ReassemblyError::Fragment(FragmentError::IndexMismatch { .. })
                )
            },
            "out-of-order error",
        )
    }

    /// Assert the number of buffered partial messages.
    ///
    /// # Errors
    /// Returns an error if the reassembler is missing or the buffered count
    /// differs from the expectation.
    pub fn assert_buffered_messages(&self, expected: usize) -> TestResult {
        let reassembler = self
            .reassembler
            .as_ref()
            .ok_or("reassembler not configured")?;
        if reassembler.buffered_len() != expected {
            return Err("unexpected buffered message count".into());
        }
        Ok(())
    }

    /// Assert that the most recent purge evicted a specific message identifier.
    ///
    /// # Errors
    /// Returns an error if the expected message identifier was not evicted.
    pub fn assert_evicted_message(&self, message_id: u64) -> TestResult {
        if !self.last_evicted.contains(&MessageId::new(message_id)) {
            return Err(format!("message {message_id} was not evicted").into());
        }
        Ok(())
    }
}
