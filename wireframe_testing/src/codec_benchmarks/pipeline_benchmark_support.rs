//! Shared vocabulary for the non-codec stages of the zero-copy programme.
//!
//! The zero-copy benchmark matrix names four stages. The codec helpers cover
//! inbound decode; the sibling modules cover the other three:
//!
//! - [`pipeline_middleware`] measures middleware pass-through through the real `from_fn` chain and
//!   `Service::call` path;
//! - [`pipeline_hooks`] measures request-hook dispatch through the client's own before-send
//!   dispatch loop;
//! - [`pipeline_outbound`] measures outbound encode through the real `encode_message_frame` seam.
//!
//! This module holds only what they share: the access-mode axis, the label
//! builder that identifies one matrix row, and the empty-measurement constant.
//!
//! Every helper in the sibling modules builds its scenario before the timer
//! starts and recycles buffers between timed iterations, so a measurement
//! reports the stage rather than the allocator. Read-only and mutating variants
//! are separate entry points because the migration's claim is about read-only
//! paths staying zero-copy, and averaging the two would erase the effect being
//! measured.

use std::time::Duration;

use super::codec_benchmark_support::{BenchmarkWorkload, Measurement};

/// Whether a measured stage only reads the frame or also mutates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// The stage inspects the frame and leaves it unchanged.
    ReadOnly,
    /// The stage rewrites at least one byte of the frame.
    Mutating,
}

impl AccessMode {
    /// Stable label segment for benchmark identifiers.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::Mutating => "mutating",
        }
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
            "hook_mutation_length_delimited_small_mutating"
        );
    }
}
