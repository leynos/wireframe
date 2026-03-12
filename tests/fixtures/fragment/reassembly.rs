//! Reassembly-focused helpers for `FragmentWorld`.

use std::{num::NonZeroUsize, time::Duration};

use wireframe_testing::reassembly::{
    FragmentReassemblyErrorExpectation,
    FragmentReassemblySnapshot,
    assert_fragment_reassembly_absent,
    assert_fragment_reassembly_buffered_messages,
    assert_fragment_reassembly_completed_len,
    assert_fragment_reassembly_error,
    assert_fragment_reassembly_evicted,
};

use super::{FragmentHeader, FragmentWorld, MessageId, Reassembler, TestResult};

impl FragmentWorld {
    fn reassembly_snapshot(&self) -> TestResult<FragmentReassemblySnapshot<'_>> {
        let reassembler = self
            .reassembler
            .as_ref()
            .ok_or("reassembler not configured")?;
        Ok(FragmentReassemblySnapshot::new(
            self.last_reassembled.as_ref(),
            self.last_reassembly_error.as_ref(),
            &self.last_evicted,
            reassembler.buffered_len(),
        ))
    }

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
        assert_fragment_reassembly_completed_len(self.reassembly_snapshot()?, expected_len)
    }

    /// Assert that no message has been fully reassembled.
    ///
    /// # Errors
    /// Returns an error if a message has already been reassembled.
    pub fn assert_no_reassembly(&self) -> TestResult {
        assert_fragment_reassembly_absent(self.reassembly_snapshot()?)
    }

    /// Assert the latest reassembly error signalled an over-limit message.
    ///
    /// # Errors
    /// Returns an error if no reassembly error was captured or it was not a
    /// message-too-large error.
    pub fn assert_reassembly_over_limit(&self) -> TestResult {
        let Some(message_id) = self
            .last_reassembly_error
            .as_ref()
            .and_then(|err| match err {
                wireframe::fragment::ReassemblyError::MessageTooLarge { message_id, .. } => {
                    Some(*message_id)
                }
                wireframe::fragment::ReassemblyError::Fragment(_) => None,
            })
        else {
            return Err("expected message-too-large reassembly error".into());
        };
        assert_fragment_reassembly_error(
            self.reassembly_snapshot()?,
            FragmentReassemblyErrorExpectation::MessageTooLarge { message_id },
        )
    }

    /// Assert that the latest reassembly error was triggered by an out-of-order
    /// fragment.
    ///
    /// # Errors
    /// Returns an error if no reassembly error was captured or it was not an
    /// index-mismatch error.
    pub fn assert_reassembly_out_of_order(&self) -> TestResult {
        assert_fragment_reassembly_error(
            self.reassembly_snapshot()?,
            FragmentReassemblyErrorExpectation::IndexMismatch,
        )
    }

    /// Assert the number of buffered partial messages.
    ///
    /// # Errors
    /// Returns an error if the reassembler is missing or the buffered count
    /// differs from the expectation.
    pub fn assert_buffered_messages(&self, expected: usize) -> TestResult {
        assert_fragment_reassembly_buffered_messages(self.reassembly_snapshot()?, expected)
    }

    /// Assert that the most recent purge evicted a specific message identifier.
    ///
    /// # Errors
    /// Returns an error if the expected message identifier was not evicted.
    pub fn assert_evicted_message(&self, message_id: u64) -> TestResult {
        assert_fragment_reassembly_evicted(self.reassembly_snapshot()?, MessageId::new(message_id))
    }
}
