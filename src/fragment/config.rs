//! Configuration used by transport-level fragmentation and reassembly.

use std::{num::NonZeroUsize, time::Duration};

use super::fragment_overhead;

/// Default per-connection cap for concurrently retained partial messages.
pub(crate) const DEFAULT_MAX_CONCURRENT_PARTIALS: usize = 64;
/// Default aggregate buffered-byte multiplier applied to the frame budget.
pub(crate) const DEFAULT_FRAGMENT_BUFFERED_BYTES_MULTIPLIER: usize = 64;

/// Settings that bound fragment sizes and reassembly resource usage.
#[derive(Clone, Copy, Debug)]
pub struct FragmentationConfig {
    /// Maximum number of logical payload bytes carried by a single fragment.
    /// The encoded fragment will additionally include marker and header
    /// overhead; the constructor ensures the final size fits within the
    /// caller's frame budget.
    pub fragment_payload_cap: NonZeroUsize,
    /// Hard cap on the fully reassembled logical message size.
    pub max_message_size: NonZeroUsize,
    /// Optional maximum number of incomplete messages retained concurrently.
    ///
    /// `None` leaves the number of partial messages unbounded.
    pub max_concurrent_partials: Option<NonZeroUsize>,
    /// Optional maximum number of payload bytes retained across all partial
    /// messages on one connection.
    ///
    /// `None` leaves the aggregate buffered-byte total unbounded.
    pub max_buffered_bytes: Option<NonZeroUsize>,
    /// Duration after which incomplete reassembly buffers are evicted.
    pub reassembly_timeout: Duration,
}

/// Guard bytes reserved for envelope framing overhead beyond the fragment body
/// and header. This accommodates length prefixes and serialization slack.
const ENVELOPE_GUARD_BYTES: usize = 32;

impl FragmentationConfig {
    /// Derive a configuration from the maximum frame body size.
    ///
    /// `frame_budget` should reflect the largest payload the transport layer
    /// will accept (for example, a length-delimited codec's `max_frame_length`).
    /// The returned configuration ensures fragment encoding overhead fits
    /// within that budget, limits concurrent partials to at most 64, and caps
    /// aggregate retained payload bytes at the larger of 64 times the frame
    /// budget and `max_message_size`.
    ///
    /// Returns `None` when the budget cannot accommodate the fixed overhead.
    #[must_use]
    pub fn for_frame_budget(
        frame_budget: usize,
        max_message_size: NonZeroUsize,
        reassembly_timeout: Duration,
    ) -> Option<Self> {
        let overhead = fragment_overhead().get();
        if frame_budget <= overhead {
            return None;
        }
        let available = frame_budget.saturating_sub(overhead + ENVELOPE_GUARD_BYTES);
        if available == 0 {
            return None;
        }
        Some(Self {
            fragment_payload_cap: NonZeroUsize::new(available)?,
            max_message_size,
            max_concurrent_partials: NonZeroUsize::new(
                frame_budget.min(DEFAULT_MAX_CONCURRENT_PARTIALS),
            ),
            max_buffered_bytes: NonZeroUsize::new(
                frame_budget
                    .saturating_mul(DEFAULT_FRAGMENT_BUFFERED_BYTES_MULTIPLIER)
                    .max(max_message_size.get()),
            ),
            reassembly_timeout,
        })
    }

    /// Convenience helper for computing the encoded fragment ceiling.
    #[must_use]
    pub fn encoded_fragment_ceiling(&self) -> usize {
        self.fragment_payload_cap.get() + fragment_overhead().get()
    }
}

#[cfg(test)]
mod tests {
    //! Verify the limits derived from frame and message budgets.

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(16 * 1024, 64 * 1024)]
    #[case(128 * 1024, 128 * 1024)]
    fn aggregate_byte_limit_covers_message_limit(
        #[case] message_limit: usize,
        #[case] expected: usize,
    ) {
        let message_limit = NonZeroUsize::new(message_limit).expect("non-zero message limit");
        let config =
            FragmentationConfig::for_frame_budget(1024, message_limit, Duration::from_secs(30))
                .expect("frame budget supports fragmentation");

        assert_eq!(config.max_buffered_bytes, NonZeroUsize::new(expected));
    }
}
