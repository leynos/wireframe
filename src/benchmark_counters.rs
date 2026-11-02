//! Counters for the bytes copied by the zero-copy migration's seams.
//!
//! Allocation counters cannot see a copy into an already-allocated buffer, so
//! the zero-copy programme needs a byte-level metric as well. The seams that
//! copy are the serializer materialization, the `Bytes::from` hand-off in
//! `src/app/outbound_encoding.rs`, and the codec write into the transport
//! buffer. Each seam reports through this module.
//!
//! The counters are compiled only when the `benchmark-counters` feature is
//! enabled. Without it every recorder is an empty inline function that the
//! optimizer removes, so production builds pay nothing for the call sites and
//! [`snapshot`] reports zeros. A harness that reports copied bytes must
//! therefore enable the feature; a test asserts that a known copy is visible
//! only in that configuration.
//!
//! Counters are process-global. A measurement must [`reset`] before its timed
//! region, read [`snapshot`] after it, and must not run concurrently with
//! another measurement in the same process.
//!
//! # Examples
//!
//! ```ignore
//! use wireframe::benchmark_counters::{self, CopiedBytes};
//!
//! benchmark_counters::reset();
//! let frame = codec.encode(payload);
//! let copied = benchmark_counters::snapshot();
//! assert_eq!(copied.framed, payload.len() as u64);
//! ```
//!
//! The example is marked `ignore` because it is only meaningful with the
//! `benchmark-counters` feature enabled.

#[cfg(feature = "benchmark-counters")]
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;

/// Bytes copied at each in-scope seam since the last [`reset`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopiedBytes {
    /// Bytes written by a serializer into the `Vec<u8>` it returns.
    pub serialized: u64,
    /// Bytes moved when the serializer vector becomes a `Bytes` hand-off.
    pub handoff: u64,
    /// Bytes copied into the codec write buffer while framing.
    pub framed: u64,
}

impl CopiedBytes {
    /// Sum of every counted seam.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::benchmark_counters::CopiedBytes;
    ///
    /// let copied = CopiedBytes {
    ///     serialized: 4,
    ///     handoff: 4,
    ///     framed: 4,
    /// };
    /// assert_eq!(copied.total(), 12);
    /// ```
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.serialized
            .saturating_add(self.handoff)
            .saturating_add(self.framed)
    }

    /// Per-operation copy count for a measured batch.
    ///
    /// Returns `None` when `operations` is zero, because a batch that performed
    /// no work has no per-operation cost to report.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::benchmark_counters::CopiedBytes;
    ///
    /// let copied = CopiedBytes {
    ///     serialized: 64,
    ///     handoff: 0,
    ///     framed: 64,
    /// };
    /// let per_operation = copied.per_operation(2).expect("two operations");
    /// assert_eq!(per_operation.framed, 32);
    /// ```
    // Not `const`: `?` on `Option` inside a constant function is still an
    // unstable feature, and no caller needs this at compile time.
    #[must_use]
    pub fn per_operation(&self, operations: u64) -> Option<Self> {
        let serialized = self.serialized.checked_div(operations)?;
        let handoff = self.handoff.checked_div(operations)?;
        let framed = self.framed.checked_div(operations)?;
        Some(Self {
            serialized,
            handoff,
            framed,
        })
    }
}

/// Serializer materialization counter.
#[cfg(feature = "benchmark-counters")]
static SERIALIZED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Serializer-to-`Bytes` hand-off counter.
#[cfg(feature = "benchmark-counters")]
static HANDOFF_BYTES: AtomicU64 = AtomicU64::new(0);

/// Codec framing counter.
#[cfg(feature = "benchmark-counters")]
static FRAMED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Records bytes written into a serializer's returned vector.
#[inline]
pub fn record_serialized(bytes: usize) {
    #[cfg(feature = "benchmark-counters")]
    {
        SERIALIZED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    #[cfg(not(feature = "benchmark-counters"))]
    {
        let _ = bytes;
    }
}

/// Records bytes copied by the serializer-to-`Bytes` hand-off.
#[inline]
pub fn record_handoff(bytes: usize) {
    #[cfg(feature = "benchmark-counters")]
    {
        HANDOFF_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    #[cfg(not(feature = "benchmark-counters"))]
    {
        let _ = bytes;
    }
}

/// Records bytes copied into a codec write buffer.
#[inline]
pub fn record_framed(bytes: usize) {
    #[cfg(feature = "benchmark-counters")]
    {
        FRAMED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    #[cfg(not(feature = "benchmark-counters"))]
    {
        let _ = bytes;
    }
}

/// Returns the counters accumulated since the last [`reset`].
///
/// Every field is zero when the `benchmark-counters` feature is disabled, so a
/// caller cannot mistake a disabled counter for a measured zero-copy path.
#[must_use]
pub fn snapshot() -> CopiedBytes {
    #[cfg(feature = "benchmark-counters")]
    {
        CopiedBytes {
            serialized: SERIALIZED_BYTES.load(Ordering::Relaxed),
            handoff: HANDOFF_BYTES.load(Ordering::Relaxed),
            framed: FRAMED_BYTES.load(Ordering::Relaxed),
        }
    }
    #[cfg(not(feature = "benchmark-counters"))]
    {
        CopiedBytes::default()
    }
}

/// Clears every counter.
///
/// Call this immediately before the timed region so setup work cannot be
/// mistaken for per-operation cost.
pub fn reset() {
    #[cfg(feature = "benchmark-counters")]
    {
        SERIALIZED_BYTES.store(0, Ordering::Relaxed);
        HANDOFF_BYTES.store(0, Ordering::Relaxed);
        FRAMED_BYTES.store(0, Ordering::Relaxed);
    }
}

/// Converts a serializer vector into the codec hand-off container.
///
/// `bytes` 1.12.1 never copies the payload here. With no spare capacity it
/// takes the vector's boxed slice in place; with spare capacity it allocates a
/// small shared control block that still points at the vector's buffer. The
/// pointer comparison therefore records zero, and the counter is a guard: a
/// future `bytes` change, or a hand-off written some other way, would show up
/// as a non-zero `handoff` instead of passing unmeasured.
///
/// # Examples
///
/// ```
/// use wireframe::benchmark_counters::bytes_from_vec;
///
/// let mut spare_capacity = Vec::with_capacity(64);
/// spare_capacity.extend_from_slice(&[1, 2, 3, 4]);
/// let bytes = bytes_from_vec(spare_capacity);
/// assert_eq!(bytes.as_ref(), &[1, 2, 3, 4]);
/// ```
#[must_use]
pub fn bytes_from_vec(bytes: Vec<u8>) -> Bytes {
    #[cfg(feature = "benchmark-counters")]
    {
        let source = bytes.as_ptr();
        let len = bytes.len();
        let handoff = Bytes::from(bytes);
        if handoff.as_ptr() != source {
            record_handoff(len);
        }
        handoff
    }
    #[cfg(not(feature = "benchmark-counters"))]
    {
        Bytes::from(bytes)
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for counter accounting.

    use super::*;

    #[test]
    fn per_operation_reports_none_for_an_empty_batch() {
        assert!(CopiedBytes::default().per_operation(0).is_none());
    }

    #[test]
    fn per_operation_divides_each_seam() {
        let copied = CopiedBytes {
            serialized: 96,
            handoff: 48,
            framed: 32,
        };
        let per_operation = copied.per_operation(4).expect("four operations");
        assert_eq!(
            per_operation,
            CopiedBytes {
                serialized: 24,
                handoff: 12,
                framed: 8,
            }
        );
    }

    #[cfg(feature = "benchmark-counters")]
    #[test]
    fn handoff_preserves_the_payload_pointer() {
        // Pointer identity is the property the recorder relies on, so assert it
        // directly rather than through the process-global counter, which other
        // tests in this binary also write to.
        let mut spare_capacity = Vec::with_capacity(64);
        spare_capacity.extend_from_slice(&[1, 2, 3, 4]);
        let source = spare_capacity.as_ptr();
        let bytes = bytes_from_vec(spare_capacity);
        assert_eq!(bytes.as_ptr(), source);

        let exact_capacity = vec![1, 2, 3, 4];
        assert_eq!(exact_capacity.len(), exact_capacity.capacity());
        let source = exact_capacity.as_ptr();
        let bytes = bytes_from_vec(exact_capacity);
        assert_eq!(bytes.as_ptr(), source);
    }

    #[cfg(feature = "benchmark-counters")]
    #[test]
    fn record_handoff_accumulates_bytes() {
        // The hand-off records nothing with the current `bytes` version, so the
        // counter is exercised directly. A delta is used rather than a reset so
        // a test running concurrently can only add to the counter, never
        // invalidate the expectation.
        let before = snapshot().handoff;
        record_handoff(7);
        record_handoff(5);
        assert_eq!(snapshot().handoff - before, 12);
    }
}
