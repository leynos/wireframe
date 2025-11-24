//! Test world for fragmentation scenarios covering both directions.
//!
//! Provides [`FragmentWorld`] to verify ordering, completion detection, and
//! error handling across the fragmentation behavioural tests while also
//! exercising outbound fragmentation by chunking payloads via the helper
//! `Fragmenter` and inspecting the resulting `FragmentBatch` state.
#![cfg(not(loom))]

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use cucumber::World;
use wireframe::fragment::{
    FragmentBatch,
    FragmentError,
    FragmentFrame,
    FragmentHeader,
    FragmentIndex,
    FragmentSeries,
    FragmentStatus,
    Fragmenter,
    MessageId,
    ReassembledMessage,
    Reassembler,
    ReassemblyError,
};

#[derive(Debug, World)]
pub struct FragmentWorld {
    series: Option<FragmentSeries>,
    last_result: Option<Result<FragmentStatus, FragmentError>>,
    fragmenter: Option<Fragmenter>,
    last_batch: Option<FragmentBatch>,
    reassembler: Option<Reassembler>,
    last_reassembled: Option<ReassembledMessage>,
    last_reassembly_error: Option<ReassemblyError>,
    now: Instant,
    last_evicted: Vec<MessageId>,
}

impl Default for FragmentWorld {
    fn default() -> Self {
        Self {
            series: None,
            last_result: None,
            fragmenter: None,
            last_batch: None,
            reassembler: None,
            last_reassembled: None,
            last_reassembly_error: None,
            now: Instant::now(),
            last_evicted: Vec::new(),
        }
    }
}

impl FragmentWorld {
    /// Start tracking a new logical message.
    pub fn start_series(&mut self, message_id: u64) {
        self.series = Some(FragmentSeries::new(MessageId::new(message_id)));
        self.last_result = None;
    }

    /// Configure a fragmenter with the provided payload cap so outbound
    /// fragmentation scenarios can chunk messages during behavioural tests.
    ///
    /// # Panics
    /// Panics if `max_payload` is zero.
    pub fn configure_fragmenter(&mut self, max_payload: usize) {
        let cap = NonZeroUsize::new(max_payload).expect("fragment cap must be non-zero");
        self.fragmenter = Some(Fragmenter::new(cap));
        self.last_batch = None;
    }

    /// Request fragmentation for a payload of `len` bytes, simulating outbound
    /// fragment production for the behavioural scenarios.
    ///
    /// # Panics
    /// Panics if [`configure_fragmenter`] has not been called yet.
    pub fn fragment_payload(&mut self, len: usize) {
        let fragmenter = self.fragmenter.as_ref().expect("fragmenter not configured");
        let payload = vec![0_u8; len];
        let batch = fragmenter
            .fragment_bytes(payload)
            .expect("fragmentation must succeed in tests");
        self.last_batch = Some(batch);
    }

    /// Force the next expected fragment index for overflow scenarios.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn force_next_index(&mut self, index: u32) {
        self.series_mut()
            .force_next_index_for_tests(FragmentIndex::new(index));
    }

    /// Feed a fragment that references the currently tracked message.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn accept_fragment(&mut self, index: u32, is_last: bool) {
        let message = self.series().message_id().get();
        self.accept_fragment_from(message, index, is_last);
    }

    /// Feed a fragment for an explicit message identifier.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn accept_fragment_from(&mut self, message: u64, index: u32, is_last: bool) {
        let header =
            FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), is_last);
        self.last_result = Some(self.series_mut().accept(header));
    }

    /// Return the most recent fragment outcome.
    ///
    /// # Panics
    /// Panics if no fragment has been processed yet.
    fn last_result(&self) -> &Result<FragmentStatus, FragmentError> {
        self.last_result
            .as_ref()
            .expect("no fragment processed yet")
    }

    fn batch(&self) -> &FragmentBatch {
        self.last_batch.as_ref().expect("no payload fragmented yet")
    }

    /// Retrieve the fragment at `index`, panicking if it is missing.
    fn get_fragment_at(&self, index: usize) -> &FragmentFrame {
        self.batch()
            .fragments()
            .get(index)
            .unwrap_or_else(|| panic!("fragment {index} missing"))
    }

    fn assert_error<F>(&self, predicate: F, expected_desc: &str)
    where
        F: FnOnce(&FragmentError) -> bool,
    {
        let err = match self.last_result() {
            Err(err) => err,
            Ok(status) => panic!("expected error but received {status:?}"),
        };
        assert!(predicate(err), "expected {expected_desc}, got {err}");
    }

    /// Assert that the latest fragment completed the logical message.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed to
    /// complete the message.
    pub fn assert_completion(&self) {
        match self.last_result() {
            Ok(FragmentStatus::Complete) => {}
            Ok(status) => panic!("unexpected status: {status:?}"),
            Err(err) => panic!("expected completion but got error: {err}"),
        }
        assert!(
            self.series().is_complete(),
            "series should be marked complete"
        );
    }

    fn series(&self) -> &FragmentSeries {
        self.series
            .as_ref()
            .expect("fragment series not initialised")
    }

    fn series_mut(&mut self) -> &mut FragmentSeries {
        self.series
            .as_mut()
            .expect("fragment series not initialised")
    }

    /// Assert that the latest fragment failed due to an index mismatch.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed for some
    /// other reason.
    pub fn assert_index_mismatch(&self) {
        self.assert_error(
            |err| matches!(err, FragmentError::IndexMismatch { .. }),
            "index mismatch",
        );
    }

    /// Assert that the latest fragment failed because the message identifier
    /// did not match the tracked series.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed for a
    /// different reason.
    pub fn assert_message_mismatch(&self) {
        self.assert_error(
            |err| matches!(err, FragmentError::MessageMismatch { .. }),
            "message mismatch",
        );
    }

    /// Assert that the latest fragment failed because the index overflowed.
    ///
    /// # Panics
    /// Panics if the series did not report an overflow.
    pub fn assert_index_overflow(&self) {
        self.assert_error(
            |err| matches!(err, FragmentError::IndexOverflow { .. }),
            "overflow error",
        );
    }

    /// Assert that the latest fragment failed because the series was already complete.
    ///
    /// # Panics
    /// Panics if the series did not report a completion error.
    pub fn assert_series_complete_error(&self) {
        self.assert_error(
            |err| matches!(err, FragmentError::SeriesComplete),
            "series completion error",
        );
    }

    /// Assert that the most recent fragmentation produced `expected` fragments
    /// for outbound fragmentation scenarios.
    ///
    /// # Panics
    /// Panics if no payload has been fragmented yet.
    pub fn assert_fragment_count(&self, expected: usize) {
        assert_eq!(self.batch().len(), expected, "unexpected fragment count");
    }

    /// Assert that the payload length of fragment `index` matches `expected`
    /// bytes for outbound fragments.
    ///
    /// # Panics
    /// Panics if no payload has been fragmented or if `index` exceeds the batch.
    pub fn assert_fragment_payload_len(&self, index: usize, expected: usize) {
        let fragment = self.get_fragment_at(index);
        assert_eq!(
            fragment.payload().len(),
            expected,
            "payload length mismatch"
        );
    }

    /// Assert that outbound fragment `index` carries the expected final flag.
    ///
    /// # Panics
    /// Panics if no payload has been fragmented or if `index` exceeds the batch.
    pub fn assert_fragment_final_flag(&self, index: usize, expected_final: bool) {
        let fragment = self.get_fragment_at(index);
        assert_eq!(
            fragment.header().is_last_fragment(),
            expected_final,
            "fragment {index} final flag mismatch",
        );
    }

    /// Assert that the outbound fragment batch carries the expected message
    /// identifier.
    ///
    /// # Panics
    /// Panics if no payload has been fragmented yet.
    pub fn assert_message_id(&self, expected: u64) {
        assert_eq!(
            self.batch().message_id(),
            MessageId::new(expected),
            "unexpected message identifier",
        );
    }

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
    pub fn push_fragment(
        &mut self,
        message_id: u64,
        index: u32,
        is_last: bool,
        payload_len: usize,
    ) {
        let reassembler = self
            .reassembler
            .as_mut()
            .expect("reassembler not configured");
        let header = FragmentHeader::new(
            MessageId::new(message_id),
            FragmentIndex::new(index),
            is_last,
        );
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
