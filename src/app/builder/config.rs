//! General configuration methods for `WireframeApp`.

use std::num::NonZeroUsize;

use super::WireframeApp;
use crate::{
    app::{
        MemoryBudgets,
        Packet,
        builder_defaults::{MAX_READ_TIMEOUT_MS, MIN_READ_TIMEOUT_MS, default_fragmentation},
    },
    codec::FrameCodec,
    fragment::FragmentationConfig,
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Configure the inbound read wake-up interval in milliseconds.
    ///
    /// Clamped between 1 and 86,400,000 milliseconds (24 h). When the interval
    /// elapses without a frame, the connection purges expired partial messages
    /// and keeps waiting: this does not close an idle connection.
    #[must_use]
    pub fn read_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.read_timeout_ms = timeout_ms.clamp(MIN_READ_TIMEOUT_MS, MAX_READ_TIMEOUT_MS);
        self
    }

    /// Override the fragmentation configuration.
    ///
    /// Provide `None` to disable fragmentation entirely.
    #[must_use]
    pub const fn fragmentation(mut self, config: Option<FragmentationConfig>) -> Self {
        self.fragmentation = config;
        self
    }

    /// Configure per-connection memory budgets for inbound buffering.
    ///
    /// These budgets apply across the inbound assembly pipeline. Message
    /// assembly uses them for per-frame rejection, and retained fragment and
    /// assembly bytes together drive soft-pressure read pacing and hard-cap
    /// connection aborts.
    #[must_use]
    pub const fn memory_budgets(mut self, budgets: MemoryBudgets) -> Self {
        self.memory_budgets = Some(budgets);
        self
    }

    /// Cap the number of message assemblies that may be in flight at once.
    ///
    /// Each connection enforces the cap when a first frame would start a new
    /// assembly. The default is 64, matching the fragment reassembler's
    /// concurrency cap. A first frame that would exceed the cap is rejected
    /// through the existing deserialization-failure policy.
    ///
    /// `NonZeroUsize::MAX` is effectively unbounded. Directly constructed
    /// [`MessageAssemblyState`](crate::message_assembler::MessageAssemblyState)
    /// values remain uncapped unless configured explicitly.
    #[must_use]
    pub const fn max_concurrent_assemblies(mut self, limit: NonZeroUsize) -> Self {
        self.max_concurrent_assemblies = Some(limit);
        self
    }

    /// Enable transport fragmentation using codec-derived defaults.
    ///
    /// The derived settings are bounded by the current frame codec budget.
    /// Call this after `with_codec` or `buffer_capacity` so defaults align with
    /// the final frame length.
    #[must_use]
    pub fn enable_fragmentation(mut self) -> Self {
        self.fragmentation = default_fragmentation(self.codec.max_frame_length());
        self
    }
}
