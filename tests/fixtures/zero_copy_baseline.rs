//! `ZeroCopyBaselineWorld` fixture for the zero-copy baseline gate.
//!
//! The world drives the same harness helpers the allocation invariants use, so
//! the scenarios and the allocator-level checks cannot disagree about which
//! stage they measure. These are harness-level checks, not application-boundary
//! tests: the chain is the benchmark chain built from the library's own
//! `Transform` and `Service` traits, and the decode drives the same seam the
//! copied-byte counter observes. The end-to-end application boundary,
//! including `forward_response`, is roadmap item `13.2.2`.
//!
//! The scenarios deliberately assert only what they can observe without an
//! instrumented allocator or a quiet process. Byte bounds live in
//! `wireframe_testing/tests/pipeline_allocation_invariants`, and the copied-byte
//! counts live in `wireframe_testing/tests/copied_bytes.rs`: those counters are
//! process-global, and the `bdd` binary runs its tests in parallel, so a
//! counter read here would race every other scenario that decodes a message.

use rstest::fixture;
use wireframe::{
    app::{Envelope, benchmark_decode_envelope},
    serializer::{BincodeSerializer, Serializer},
};
/// Shared result type used by the baseline fixture and steps.
pub use wireframe_testing::TestResult;
use wireframe_testing::codec_benchmarks::{
    AccessMode,
    BenchmarkWorkload,
    CodecUnderTest,
    PayloadClass,
    StageOutcome,
    middleware_payload_handoff,
    payload_for_class,
};

/// Behavioural world for the zero-copy baseline scenarios.
pub struct ZeroCopyBaselineWorld {
    /// Runtime the middleware helper needs; the scenario's shared thread.
    runtime: tokio::runtime::Runtime,
    /// Workload the scenarios build their payload from.
    payload_class: PayloadClass,
    /// What the last middleware pass did to the payload buffer.
    stage: Option<StageOutcome>,
    /// Payload, consumed length and frame length from the last decode.
    decoded: Option<DecodedFrame>,
}

/// What one pass through the benchmark decode seam returned.
struct DecodedFrame {
    /// Payload bytes the decoded envelope carries.
    payload: Vec<u8>,
    /// Bytes of the frame the decoder reports it consumed.
    consumed: usize,
    /// Length of the frame the decoder was handed.
    frame_len: usize,
}

/// What a scenario expects a stage to do with the payload buffer.
#[derive(Debug, Clone, Copy)]
enum BufferExpectation {
    /// The stage must hand on the buffer it received.
    Preserved,
    /// The stage must allocate a replacement buffer.
    Relocated,
}

/// Build the zero-copy baseline world on a current-thread runtime.
///
/// # Errors
///
/// Returns the runtime construction failure; a scenario cannot run without one.
#[fixture]
pub fn zero_copy_baseline_world() -> TestResult<ZeroCopyBaselineWorld> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| format!("zero-copy baseline fixture setup failed: {error}"))?;
    Ok(ZeroCopyBaselineWorld {
        runtime,
        payload_class: PayloadClass::Large,
        stage: None,
        decoded: None,
    })
}

impl ZeroCopyBaselineWorld {
    /// Builds the workload the scenarios measure.
    const fn workload(&self) -> BenchmarkWorkload {
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: self.payload_class,
        }
    }

    /// Records that the next scenario carries a large payload.
    pub fn arrive_with_large_payload(&mut self) { self.payload_class = PayloadClass::Large; }

    /// Drives one pass through the read-only middleware chain.
    pub fn pass_through_read_only_middleware(&mut self) {
        self.stage = Some(self.runtime.block_on(middleware_payload_handoff(
            self.workload(),
            AccessMode::ReadOnly,
        )));
    }

    /// Drives one pass through the middleware that copies its frame.
    pub fn pass_through_copying_middleware(&mut self) {
        self.stage = Some(self.runtime.block_on(middleware_payload_handoff(
            self.workload(),
            AccessMode::Copying,
        )));
    }

    /// The last middleware pass, or the error a scenario without one reports.
    ///
    /// # Errors
    ///
    /// Returns an error when no pass has run, so a scenario cannot assert on a
    /// stage it never drove.
    fn last_stage(&self) -> TestResult<&StageOutcome> {
        self.stage
            .as_ref()
            .ok_or("no middleware pass was performed")
            .map_err(Into::into)
    }

    /// Asserts the stage did what `expectation` states with the payload buffer.
    ///
    /// Both buffer assertions are the same question with opposite answers, so
    /// the error message reports the addresses the stage actually used rather
    /// than only the outcome that was wanted.
    ///
    /// # Errors
    ///
    /// Returns an error when the stage did the opposite of `expectation`.
    fn assert_buffer_outcome(&self, expectation: BufferExpectation) -> TestResult {
        let stage = self.last_stage()?;
        let is_preserved = stage.is_payload_preserved();
        match expectation {
            BufferExpectation::Preserved if is_preserved => Ok(()),
            BufferExpectation::Relocated if !is_preserved => Ok(()),
            BufferExpectation::Preserved => Err(format!(
                "the payload buffer moved: {} -> {}",
                stage.arrival_address, stage.departure_address
            )
            .into()),
            BufferExpectation::Relocated => Err(format!(
                "a copying middleware kept the payload buffer at {}",
                stage.departure_address
            )
            .into()),
        }
    }

    /// Asserts the stage returned the buffer it was handed.
    ///
    /// # Errors
    ///
    /// Returns an error if the stage relocated the payload buffer.
    pub fn assert_payload_buffer_preserved(&self) -> TestResult {
        self.assert_buffer_outcome(BufferExpectation::Preserved)
    }

    /// Asserts the payload bytes are unchanged after the pass.
    ///
    /// # Errors
    ///
    /// Returns an error if the stage rewrote the payload.
    pub fn assert_payload_unchanged(&self) -> TestResult {
        let stage = self.last_stage()?;
        let expected = payload_for_class(self.payload_class);
        if stage.payload.as_slice() != expected {
            return Err("the read-only chain rewrote the payload".into());
        }
        Ok(())
    }

    /// Asserts the stage relocated the payload buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the stage kept the buffer it was handed.
    pub fn assert_payload_buffer_relocated(&self) -> TestResult {
        self.assert_buffer_outcome(BufferExpectation::Relocated)
    }

    /// Encodes an envelope, then decodes it through the benchmark decode seam.
    ///
    /// # Errors
    ///
    /// Returns the serializer or decoder failure the seam reports.
    pub fn decode_framed_message(&mut self) -> TestResult {
        let envelope = Envelope::new(1, Some(7), payload_for_class(self.payload_class).to_vec());
        let framed = BincodeSerializer.serialize(&envelope)?;
        let frame_len = framed.len();

        let (decoded, consumed) = benchmark_decode_envelope(&BincodeSerializer, &framed)?;
        self.decoded = Some(DecodedFrame {
            payload: decoded.payload_bytes().to_vec(),
            consumed,
            frame_len,
        });
        Ok(())
    }

    /// The last decode, or the error a scenario without one reports.
    ///
    /// # Errors
    ///
    /// Returns an error when no decode has run, so a scenario cannot assert on
    /// a frame it never decoded.
    fn last_decode(&self) -> TestResult<&DecodedFrame> {
        self.decoded
            .as_ref()
            .ok_or("no decode was performed")
            .map_err(Into::into)
    }

    /// Asserts the decoded payload is the payload the frame carried.
    ///
    /// # Errors
    ///
    /// Returns an error if no decode ran, or if the payload differs.
    pub fn assert_decoded_payload_matches(&self) -> TestResult {
        let decoded = self.last_decode()?;
        let expected = payload_for_class(self.payload_class);
        if decoded.payload.as_slice() != expected {
            return Err(format!(
                "the decoded payload is {} bytes; the frame carried {}",
                decoded.payload.len(),
                expected.len()
            )
            .into());
        }
        Ok(())
    }

    /// Asserts the decoder consumed the whole frame it was handed.
    ///
    /// # Errors
    ///
    /// Returns an error if no decode ran, or if bytes were left unconsumed.
    pub fn assert_consumed_whole_frame(&self) -> TestResult {
        let decoded = self.last_decode()?;
        if decoded.consumed != decoded.frame_len {
            return Err(format!(
                "the decoder consumed {} of {} frame bytes",
                decoded.consumed, decoded.frame_len
            )
            .into());
        }
        Ok(())
    }
}
