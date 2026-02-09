//! Defaults and helpers for Wireframe application configuration.

use std::{num::NonZeroUsize, time::Duration};

use crate::{codec::clamp_frame_length, fragment::FragmentationConfig};

pub(super) const MIN_READ_TIMEOUT_MS: u64 = 1;
pub(super) const MAX_READ_TIMEOUT_MS: u64 = 86_400_000;
/// Default preamble read timeout in milliseconds.
pub(super) const DEFAULT_READ_TIMEOUT_MS: u64 = 100;
const DEFAULT_FRAGMENT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MESSAGE_SIZE_MULTIPLIER: usize = 16;

pub(super) fn default_fragmentation(frame_budget: usize) -> Option<FragmentationConfig> {
    let frame_budget = clamp_frame_length(frame_budget);
    let max_message =
        NonZeroUsize::new(frame_budget.saturating_mul(DEFAULT_MESSAGE_SIZE_MULTIPLIER))
            .or_else(|| NonZeroUsize::new(frame_budget));
    max_message.and_then(|limit| {
        FragmentationConfig::for_frame_budget(frame_budget, limit, DEFAULT_FRAGMENT_TIMEOUT)
    })
}
