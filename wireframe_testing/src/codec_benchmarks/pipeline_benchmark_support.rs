//! Shared vocabulary for the non-codec stages of the zero-copy programme.
//!
//! The zero-copy benchmark matrix names four stages. The codec helpers cover
//! inbound decode; the sibling modules cover the other three:
//!
//! - [`pipeline_middleware`](super::pipeline_middleware) measures middleware pass-through through a
//!   chain built from the library's `Transform` and `Service` traits, mirroring `FnService`
//!   dispatch. It does not use `from_fn`, which cannot accept a middleware that forwards to
//!   `next.call`;
//! - [`pipeline_hooks`](super::pipeline_hooks) measures request-hook dispatch through the client's
//!   own before-send dispatch loop;
//! - [`pipeline_outbound`](super::pipeline_outbound) measures outbound encode through the real
//!   `encode_message_frame` seam.
//!
//! This module holds only what they share: the access-mode axis, the label
//! builder that identifies one matrix row, and the empty-measurement constant.
//!
//! Every helper in the sibling modules builds its scenario before the timer
//! starts and recycles buffers between timed iterations, so a measurement
//! reports the stage rather than the allocator. Read-only and mutating variants
//! are separate entry points because the migration's claim is about read-only
//! paths staying zero-copy, and averaging the two would erase the effect being
//! measured. A third variant, [`AccessMode::Copying`], is a seeded fault rather
//! than a workload: it exists so the invariants that reject a copying stage can
//! be shown to fail when one is present.

use std::time::Duration;

use super::codec_benchmark_support::{BenchmarkWorkload, Measurement};

/// Whether a measured stage only reads the frame or also mutates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// The stage inspects the frame and leaves it unchanged.
    ReadOnly,
    /// The stage rewrites at least one byte of the frame.
    Mutating,
    /// Seeded fault: the stage deliberately copies the frame.
    ///
    /// This is not a workload the matrix captures. It exists so the read-only
    /// invariants can show that they reject a copying stage: a bound that no
    /// seeded fault can break proves nothing about the production path. Never
    /// include it in a timed row.
    Copying,
}

impl AccessMode {
    /// Stable label segment for benchmark identifiers.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::Mutating => "mutating",
            Self::Copying => "copying",
        }
    }
}

/// What one pass through a stage did to the payload buffer.
///
/// Addresses are reported as `usize` rather than pointers so the value is a
/// plain, comparable number: the question is only whether the buffer the stage
/// returned is the one it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageOutcome {
    /// Address of the payload buffer when the stage received it.
    pub arrival_address: usize,
    /// Address of the payload buffer when the stage returned it.
    pub departure_address: usize,
    /// Payload bytes the stage returned.
    pub payload: Vec<u8>,
}

impl StageOutcome {
    /// Whether the stage returned the buffer it was given, unrelocated.
    ///
    /// A stage that moves the payload keeps the address; a stage that copies it
    /// necessarily allocates a different one, because the original is still
    /// live while the copy is made.
    #[must_use]
    pub const fn is_payload_preserved(&self) -> bool {
        self.arrival_address == self.departure_address
    }
}

/// Empty measurement, used when a caller asks for zero iterations.
pub(super) const fn empty_measurement() -> Measurement {
    Measurement {
        operations: 0,
        payload_bytes: 0,
        elapsed: Duration::ZERO,
    }
}

/// Builds the stable benchmark identifier for one pipeline row.
///
/// The identifier names the stage, codec, payload class, and access mode so a
/// threshold can be attached to one row rather than an average across rows.
///
/// # Examples
///
/// ```
/// use wireframe_testing::codec_benchmarks::{
///     AccessMode,
///     BenchmarkWorkload,
///     CodecUnderTest,
///     PayloadClass,
///     pipeline_label,
/// };
///
/// let workload = BenchmarkWorkload {
///     codec: CodecUnderTest::Hotline,
///     payload_class: PayloadClass::Large,
/// };
/// assert_eq!(
///     pipeline_label("middleware_passthrough", workload, AccessMode::ReadOnly),
///     "middleware_passthrough_hotline_large_readonly"
/// );
/// ```
#[must_use]
pub fn pipeline_label(stage: &str, workload: BenchmarkWorkload, mode: AccessMode) -> String {
    format!("{}_{}", workload_label(stage, workload), mode.label())
}

/// Builds the stable benchmark identifier for a stage that has no access mode.
///
/// The outbound encode stage always writes, so an access-mode segment would
/// claim a distinction the stage does not make. Its two passes are identified
/// by stage name instead.
///
/// # Examples
///
/// ```
/// use wireframe_testing::codec_benchmarks::{
///     BenchmarkWorkload,
///     CodecUnderTest,
///     PayloadClass,
///     workload_label,
/// };
///
/// let workload = BenchmarkWorkload {
///     codec: CodecUnderTest::Hotline,
///     payload_class: PayloadClass::Small,
/// };
/// assert_eq!(
///     workload_label("outbound_serialize", workload),
///     "outbound_serialize_hotline_small"
/// );
/// ```
#[must_use]
pub fn workload_label(stage: &str, workload: BenchmarkWorkload) -> String {
    format!(
        "{stage}_{}_{}",
        workload.codec.label(),
        workload.payload_class.label()
    )
}

#[cfg(test)]
mod tests {
    //! Tests for the shared label vocabulary.

    use super::*;
    use crate::codec_benchmarks::{CodecUnderTest, PayloadClass};

    /// Small length-delimited workload used by the label tests.
    fn small_workload() -> BenchmarkWorkload {
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: PayloadClass::Small,
        }
    }

    #[test]
    fn pipeline_label_names_every_dimension() {
        assert_eq!(
            pipeline_label("hook_mutation", small_workload(), AccessMode::Mutating),
            "hook_mutation_length_delimited_small_mutating",
            "pipeline_label(\"hook_mutation\", small_workload(), AccessMode::Mutating) should \
             equal \"hook_mutation_length_delimited_small_mutating\""
        );
    }
}
