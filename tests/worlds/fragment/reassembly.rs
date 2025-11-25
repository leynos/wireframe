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

impl FragmentWorld {
    /// Configure a reassembler with size and timeout guards.
    ///
    /// # Panics
    /// Panics if `max_message_size` is zero.
    pub fn configure_reassembler(&mut self, max_message_size: usize, timeout_secs: u64) {
        let size = NonZeroUsize::new(max_message_size).expect("reassembly cap must be non-zero");
        self.reassembler = Some(Reassembler::new(size, Duration::from_secs(timeout_secs)));
        self.last_reassembled = None;
        self.last_reassembly_error = None;
        self.last_evicted.clear();
    }

    /// Submit a fragment to the configured reassembler.
    ///
    /// # Panics
    /// Panics if the reassembler has not been configured.
    pub fn push_fragment(&mut self, header: FragmentHeader, payload_len: usize) {
        let reassembler = self
            .reassembler
            .as_mut()
            .expect("reassembler not configured");
        let payload = vec![0_u8; payload_len];
        self.last_reassembly_error = None;
        self.last_reassembled = None;
        match reassembler.push_at(header, payload, self.now) {
            Ok(output) => self.last_reassembled = output,
            Err(err) => self.last_reassembly_error = Some(err),
        }
    }

    /// Advance the simulated clock.
    ///
    /// # Panics
    ///
    /// Panics if advancing the clock would overflow [`Instant`].
    pub fn advance_time(&mut self, delta: Duration) {
        self.now = self
            .now
            .checked_add(delta)
            .expect("time advance overflowed");
    }

    /// Purge expired partial messages based on the current clock reading.
    ///
    /// # Panics
    ///
    /// Panics if the reassembler has not been configured.
    pub fn purge_reassembly(&mut self) {
        let reassembler = self
            .reassembler
            .as_mut()
            .expect("reassembler not configured");
        self.last_evicted = reassembler.purge_expired_at(self.now);
    }

    /// Assert that a message has been reassembled with the expected payload length.
    ///
    /// # Panics
    /// Panics if no message has been reassembled yet.
    pub fn assert_reassembled_len(&self, expected_len: usize) {
        let message = self
            .last_reassembled
            .as_ref()
            .expect("no message reassembled");
        assert_eq!(
            message.payload().len(),
            expected_len,
            "payload length mismatch"
        );
    }

    /// Assert that no message has been fully reassembled.
    ///
    /// # Panics
    ///
    /// Panics if a message has already been reassembled.
    pub fn assert_no_reassembly(&self) {
        assert!(
            self.last_reassembled.is_none(),
            "unexpected reassembled message present"
        );
    }

    /// Assert the latest reassembly error signalled an over-limit message.
    ///
    /// # Panics
    ///
    /// Panics if no reassembly error was captured.
    pub fn assert_reassembly_over_limit(&self) {
        let err = self
            .last_reassembly_error
            .as_ref()
            .expect("no reassembly error captured");
        assert!(
            matches!(err, ReassemblyError::MessageTooLarge { .. }),
            "expected message-too-large error, got {err}"
        );
    }

    /// Assert that the latest reassembly error was triggered by an out-of-order fragment.
    ///
    /// # Panics
    ///
    /// Panics if no reassembly error was captured or if the error was not an index mismatch.
    pub fn assert_reassembly_out_of_order(&self) {
        let err = self
            .last_reassembly_error
            .as_ref()
            .expect("no reassembly error captured");
        assert!(
            matches!(
                err,
                ReassemblyError::Fragment(FragmentError::IndexMismatch { .. })
            ),
            "expected out-of-order error, got {err}"
        );
    }

    /// Assert the number of buffered partial messages.
    ///
    /// # Panics
    /// Panics if the reassembler has not been configured.
    pub fn assert_buffered_messages(&self, expected: usize) {
        let reassembler = self
            .reassembler
            .as_ref()
            .expect("reassembler not configured");
        assert_eq!(
            reassembler.buffered_len(),
            expected,
            "unexpected buffered message count"
        );
    }

    /// Assert that the most recent purge evicted a specific message identifier.
    ///
    /// # Panics
    ///
    /// Panics if the purge record does not contain `message_id`.
    pub fn assert_evicted_message(&self, message_id: u64) {
        assert!(
            self.last_evicted.contains(&MessageId::new(message_id)),
            "message {message_id} was not evicted"
        );
    }
}
