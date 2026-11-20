//! `ZeroCopyBaselineWorld` fixture for the zero-copy baseline gate.
//!
//! The world drives the same harness helpers the allocation invariants use, so
//! the behavioural scenarios and the allocator-level checks cannot disagree
//! about which stage they measure. It deliberately asserts only what a scenario
//! can observe without an instrumented allocator; byte bounds live in
//! `wireframe_testing/tests/pipeline_allocation_invariants`.

use rstest::fixture;
use wireframe::{
    app::{Envelope, benchmark_decode_envelope},
    benchmark_counters,
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
    /// Deserialized bytes the last decode recorded.
    deserialized_bytes: Option<u64>,
    /// Payload length the last decode produced.
    decoded_payload_bytes: Option<u64>,
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
        deserialized_bytes: None,
        decoded_payload_bytes: None,
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

    /// Encodes an envelope, then decodes it through the production seam.
    ///
    /// # Errors
    ///
    /// Returns the serializer or decoder failure the seam reports.
    pub fn decode_framed_message(&mut self) -> TestResult {
        let envelope = Envelope::new(1, Some(7), payload_for_class(self.payload_class).to_vec());
        let framed = BincodeSerializer.serialize(&envelope)?;

        benchmark_counters::reset();
        let (decoded, _consumed) = benchmark_decode_envelope(&BincodeSerializer, &framed)?;

        self.deserialized_bytes = Some(benchmark_counters::snapshot().deserialized);
        self.decoded_payload_bytes = Some(decoded.payload_bytes().len() as u64);
        Ok(())
    }

    /// Asserts one payload copy was recorded for the decoded length.
    ///
    /// # Errors
    ///
    /// Returns an error if no decode ran, or if the recorded count differs from
    /// the payload length the decode produced.
    pub fn assert_deserialized_equals_payload(&self) -> TestResult {
        let recorded = self.deserialized_bytes.ok_or("no decode was performed")?;
        let payload = self
            .decoded_payload_bytes
            .ok_or("no decode was performed")?;
        if recorded != payload {
            return Err(
                format!("expected {payload} deserialized bytes, recorded {recorded}").into(),
            );
        }
        Ok(())
    }
}
