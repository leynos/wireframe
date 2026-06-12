//! Fragmentation configuration builders for integration tests.

use std::{num::NonZeroUsize, time::Duration};

use wireframe::fragment::FragmentationConfig;

use super::{TestError, TestResult};

const MESSAGE_LIMIT_MULTIPLIER: usize = 16;

/// Create a fragmentation config for a given buffer capacity.
///
/// Uses [`MESSAGE_LIMIT_MULTIPLIER`] times capacity as the message limit and a
/// 30ms reassembly timeout.
///
/// # Errors
///
/// Returns an error if the message limit overflows or if the frame budget
/// is too small to accommodate fragment overhead.
pub fn fragmentation_config(capacity: usize) -> TestResult<FragmentationConfig> {
    let message_limit = capacity
        .checked_mul(MESSAGE_LIMIT_MULTIPLIER)
        .and_then(NonZeroUsize::new)
        .ok_or(TestError::Setup("message limit overflow or zero"))?;

    let config =
        FragmentationConfig::for_frame_budget(capacity, message_limit, Duration::from_millis(30))
            .ok_or(TestError::Setup(
            "frame budget must exceed fragment overhead",
        ))?;

    Ok(config)
}

/// Create a fragmentation config with a custom reassembly timeout.
///
/// # Errors
///
/// Returns an error if the underlying [`fragmentation_config`] fails.
pub fn fragmentation_config_with_timeout(
    capacity: usize,
    timeout_ms: u64,
) -> TestResult<FragmentationConfig> {
    let mut config = fragmentation_config(capacity)?;
    config.reassembly_timeout = Duration::from_millis(timeout_ms);
    Ok(config)
}
