#![cfg(not(loom))]
//! Test world validating fragment ordering, completion, and error semantics.

use cucumber::World;
use wireframe::fragment::{
    FragmentError,
    FragmentHeader,
    FragmentIndex,
    FragmentSeries,
    FragmentStatus,
    MessageId,
};

#[derive(Debug, Default, World)]
pub struct FragmentWorld {
    series: Option<FragmentSeries>,
    last_result: Option<Result<FragmentStatus, FragmentError>>,
}

impl FragmentWorld {
    /// Start tracking a new logical message.
    pub fn start_series(&mut self, message_id: u64) {
        self.series = Some(FragmentSeries::new(MessageId::new(message_id)));
        self.last_result = None;
    }

    /// Force the next expected fragment index for overflow scenarios.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn force_next_index(&mut self, index: u32) {
        let series = self
            .series
            .as_mut()
            .expect("fragment series not initialised");
        series.force_next_index_for_tests(FragmentIndex::new(index));
    }

    /// Feed a fragment that references the currently tracked message.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn accept_fragment(&mut self, index: u32, is_last: bool) {
        let message = self
            .series
            .as_ref()
            .expect("fragment series not initialised")
            .message_id()
            .get();
        self.accept_fragment_from(message, index, is_last);
    }

    /// Feed a fragment for an explicit message identifier.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn accept_fragment_from(&mut self, message: u64, index: u32, is_last: bool) {
        let header =
            FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), is_last);
        let series = self
            .series
            .as_mut()
            .expect("fragment series not initialised");
        self.last_result = Some(series.accept(header));
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
        let series = self.series.as_ref().expect("series missing");
        assert!(series.is_complete(), "series should be marked complete");
    }

    /// Assert that the latest fragment failed due to an index mismatch.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed for some
    /// other reason.
    pub fn assert_index_mismatch(&self) {
        self.assert_error(|err| matches!(err, FragmentError::IndexMismatch { .. }), "index mismatch");
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
        self.assert_error(|err| matches!(err, FragmentError::IndexOverflow { .. }), "overflow error");
    }

    /// Assert that the latest fragment failed because the series was already complete.
    ///
    /// # Panics
    /// Panics if the series did not report a completion error.
    pub fn assert_series_complete_error(&self) {
        self.assert_error(|err| matches!(err, FragmentError::SeriesComplete), "series completion error");
    }
}
