//! Assertion helpers for fragment integration tests.

use tokio::{
    sync::mpsc,
    time::{Duration as TokioDuration, timeout},
};

use super::{TestError, TestResult};

/// Assert that the handler received the expected payload.
///
/// # Errors
///
/// Returns an error if the receive times out, the channel is closed,
/// or the observed payload does not match the expected payload.
pub async fn assert_handler_observed(
    rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    expected: &[u8],
) -> TestResult<()> {
    let observed = timeout(TokioDuration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    if observed != expected {
        return Err(TestError::Assertion(format!(
            "observed payload mismatch: expected {expected:?}, got {observed:?}"
        )));
    }
    Ok(())
}
