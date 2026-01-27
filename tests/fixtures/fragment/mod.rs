//! `FragmentWorld` fixture for rstest-bdd tests.
//!
//! Tracks fragmentation and reassembly state for fragment scenarios.

mod reassembly;

use std::{num::NonZeroUsize, time::Instant};

use rstest::fixture;
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

// Re-export TestResult from common for use in steps
pub use crate::common::TestResult;

/// Test world tracking fragmentation state across behavioural scenarios.
#[derive(Debug)]
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

// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn fragment_world() -> FragmentWorld {
    FragmentWorld::default()
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
    /// # Errors
    /// Returns an error if the payload cap is zero.
    pub fn configure_fragmenter(&mut self, max_payload: usize) -> TestResult {
        let cap = NonZeroUsize::new(max_payload).ok_or("fragment cap must be non-zero")?;
        self.fragmenter = Some(Fragmenter::new(cap));
        self.last_batch = None;
        Ok(())
    }

    /// Request fragmentation for a payload of `len` bytes, simulating outbound
    /// fragment production for the behavioural scenarios.
    ///
    /// # Errors
    /// Returns an error if the fragmenter is missing or fragmentation fails.
    pub fn fragment_payload(&mut self, len: usize) -> TestResult {
        let fragmenter = self
            .fragmenter
            .as_ref()
            .ok_or("fragmenter not configured")?;
        let payload = vec![0_u8; len];
        let batch = fragmenter.fragment_bytes(payload)?;
        self.last_batch = Some(batch);
        Ok(())
    }

    /// Force the next expected fragment index for overflow scenarios.
    ///
    /// # Errors
    /// Returns an error if a fragment series has not been initialised.
    pub fn force_next_index(&mut self, index: u32) -> TestResult {
        self.series_mut()?
            .force_next_index_for_tests(FragmentIndex::new(index));
        Ok(())
    }

    /// Feed a fragment that references the currently tracked message.
    ///
    /// # Errors
    /// Returns an error if no fragment series has been initialised.
    pub fn accept_fragment(&mut self, index: u32, is_last: bool) -> TestResult {
        let message = self.series()?.message_id().get();
        self.accept_fragment_from(message, index, is_last)
    }

    /// Feed a fragment for an explicit message identifier.
    ///
    /// # Errors
    /// Returns an error if no fragment series has been initialised.
    pub fn accept_fragment_from(&mut self, message: u64, index: u32, is_last: bool) -> TestResult {
        let header =
            FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), is_last);
        self.last_result = Some(self.series_mut()?.accept(header));
        Ok(())
    }

    /// Return the most recent fragment outcome.
    fn last_result(&self) -> TestResult<&Result<FragmentStatus, FragmentError>> {
        self.last_result
            .as_ref()
            .ok_or_else(|| "no fragment processed yet".into())
    }

    fn batch(&self) -> TestResult<&FragmentBatch> {
        self.last_batch
            .as_ref()
            .ok_or_else(|| "no payload fragmented yet".into())
    }

    /// Retrieve the fragment at `index`.
    fn get_fragment_at(&self, index: usize) -> TestResult<&FragmentFrame> {
        let fragment = self
            .batch()?
            .fragments()
            .get(index)
            .ok_or_else(|| format!("fragment {index} missing"))?;
        Ok(fragment)
    }

    fn assert_error<F>(&self, predicate: F, expected_desc: &str) -> TestResult
    where
        F: FnOnce(&FragmentError) -> bool,
    {
        let err = match self.last_result()? {
            Err(err) => err,
            Ok(status) => return Err(format!("expected error but received {status:?}").into()),
        };
        if !predicate(err) {
            return Err(format!("expected {expected_desc}, got {err}").into());
        }
        Ok(())
    }

    /// Assert that the latest fragment completed the logical message.
    ///
    /// # Errors
    /// Returns an error if the fragment did not complete the message or no
    /// fragment was processed.
    pub fn assert_completion(&self) -> TestResult {
        match self.last_result()? {
            Ok(FragmentStatus::Complete) => {}
            Ok(status) => return Err(format!("unexpected status: {status:?}").into()),
            Err(err) => return Err(format!("expected completion but got error: {err}").into()),
        }
        if !self.series()?.is_complete() {
            return Err("series should be marked complete".into());
        }
        Ok(())
    }

    fn series(&self) -> TestResult<&FragmentSeries> {
        self.series
            .as_ref()
            .ok_or_else(|| "fragment series not initialised".into())
    }

    fn series_mut(&mut self) -> TestResult<&mut FragmentSeries> {
        self.series
            .as_mut()
            .ok_or_else(|| "fragment series not initialised".into())
    }

    /// Assert that the latest fragment failed due to an index mismatch.
    ///
    /// # Errors
    /// Returns an error if the last fragment result does not indicate an index
    /// mismatch or no fragment was processed.
    pub fn assert_index_mismatch(&self) -> TestResult {
        self.assert_error(
            |err| matches!(err, FragmentError::IndexMismatch { .. }),
            "index mismatch",
        )
    }

    /// Assert that the latest fragment failed because the message identifier
    /// did not match the tracked series.
    ///
    /// # Errors
    /// Returns an error if the last fragment result is not a message mismatch
    /// or no fragment was processed.
    pub fn assert_message_mismatch(&self) -> TestResult {
        self.assert_error(
            |err| matches!(err, FragmentError::MessageMismatch { .. }),
            "message mismatch",
        )
    }

    /// Assert that the latest fragment failed because the index overflowed.
    ///
    /// # Errors
    /// Returns an error if the last fragment result is not an overflow or no
    /// fragment was processed.
    pub fn assert_index_overflow(&self) -> TestResult {
        self.assert_error(
            |err| matches!(err, FragmentError::IndexOverflow { .. }),
            "overflow error",
        )
    }

    /// Assert that the latest fragment failed because the series was already
    /// complete.
    ///
    /// # Errors
    /// Returns an error if the last fragment result is not a completion error
    /// or no fragment was processed.
    pub fn assert_series_complete_error(&self) -> TestResult {
        self.assert_error(
            |err| matches!(err, FragmentError::SeriesComplete),
            "series completion error",
        )
    }

    /// Assert that the most recent fragmentation produced `expected` fragments
    /// for outbound fragmentation scenarios.
    ///
    /// # Errors
    /// Returns an error if no batch exists or the fragment count mismatches.
    pub fn assert_fragment_count(&self, expected: usize) -> TestResult {
        let actual = self.batch()?.len();
        if actual != expected {
            return Err(format!("expected {expected} fragments, got {actual}").into());
        }
        Ok(())
    }

    /// Assert that the payload length of fragment `index` matches `expected`
    /// bytes for outbound fragments.
    ///
    /// # Errors
    /// Returns an error if the batch is missing or the payload length differs.
    pub fn assert_fragment_payload_len(&self, index: usize, expected: usize) -> TestResult {
        let fragment = self.get_fragment_at(index)?;
        let actual = fragment.payload().len();
        if actual != expected {
            return Err(format!(
                "fragment {index} payload length mismatch: expected {expected}, got {actual}"
            )
            .into());
        }
        Ok(())
    }

    /// Assert that outbound fragment `index` carries the expected final flag.
    ///
    /// # Errors
    /// Returns an error if the batch is missing or the final flag mismatches.
    pub fn assert_fragment_final_flag(&self, index: usize, expected_final: bool) -> TestResult {
        let fragment = self.get_fragment_at(index)?;
        let actual = fragment.header().is_last_fragment();
        if actual != expected_final {
            return Err(format!(
                "fragment {index} final flag mismatch: expected {expected_final}, got {actual}"
            )
            .into());
        }
        Ok(())
    }

    /// Assert that the outbound fragment batch carries the expected message
    /// identifier.
    ///
    /// # Errors
    /// Returns an error if the batch is missing or the message id differs from
    /// the expectation.
    pub fn assert_message_id(&self, expected: u64) -> TestResult {
        let actual = self.batch()?.message_id();
        let expected_id = MessageId::new(expected);
        if actual != expected_id {
            return Err(format!(
                "unexpected message identifier: expected {expected_id:?}, got {actual:?}"
            )
            .into());
        }
        Ok(())
    }
}
