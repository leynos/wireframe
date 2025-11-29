//! Configuration used by transport-level fragmentation and reassembly.

use std::{num::NonZeroUsize, time::Duration};

use super::fragment_overhead;

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
    /// Duration after which incomplete reassembly buffers are evicted.
    pub reassembly_timeout: Duration,
}

/// Guard bytes reserved for envelope framing overhead beyond the fragment body
/// and header. This accommodates length prefixes and serialisation slack.
const ENVELOPE_GUARD_BYTES: usize = 32;

impl FragmentationConfig {
    /// Derive a configuration from the maximum frame body size.
    ///
    /// `frame_budget` should reflect the largest payload the transport layer
    /// will accept (for example, a length-delimited codec's `max_frame_length`).
    /// The returned configuration ensures fragment encoding overhead fits
    /// within that budget.
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
            reassembly_timeout,
        })
    }

    /// Convenience helper for computing the encoded fragment ceiling.
    #[must_use]
    pub fn encoded_fragment_ceiling(&self) -> usize {
        self.fragment_payload_cap.get() + fragment_overhead().get()
    }
}
