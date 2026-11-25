//! Terminal outcome bookkeeping for a running server handle.
//!
//! The three functions here are pure and total, so the handle's decision
//! logic is enumerable: [`classify_drain`] turns one bounded drain into an
//! outcome and a caller-facing result, [`merge_outcome`] folds that outcome
//! into the handle's record, and [`drain_bound`] chooses the bound for the
//! next attempt.

use std::time::Duration;

use tokio::time::error::Elapsed;
use wireframe::server::ServerError;

use crate::{TestError, TestResult};

/// How a [`RunningWireframeServer`]'s shutdown attempts have concluded.
///
/// [`RunningWireframeServer`]: super::RunningWireframeServer
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownOutcome {
    /// The server drained: accept loops exited and the listener is released.
    Completed,
    /// The server reported `ServerError::AbnormalTermination`.
    AbnormalTermination,
    /// Shutdown was requested but the drain did not finish within the bound.
    ///
    /// Not terminal: a later `shutdown` call may still observe completion.
    TimedOut,
}

/// Classify one bounded drain into an outcome and the caller-facing result.
///
/// The elapsed case carries the original [`Elapsed`] so callers keep the
/// timeout's own diagnostics rather than a reconstructed message.
pub(crate) fn classify_drain(
    drained: Result<Result<(), ServerError>, Elapsed>,
) -> (ShutdownOutcome, TestResult) {
    match drained {
        Ok(Ok(())) => (ShutdownOutcome::Completed, Ok(())),
        Ok(Err(error)) => (
            ShutdownOutcome::AbnormalTermination,
            Err(TestError::Server(error)),
        ),
        Err(elapsed) => (ShutdownOutcome::TimedOut, Err(TestError::Timeout(elapsed))),
    }
}

/// Fold a newly observed outcome into the handle's record.
///
/// [`ShutdownOutcome::Completed`] and
/// [`ShutdownOutcome::AbnormalTermination`] are absorbing, so a later
/// observation can never overwrite a terminal result.
/// [`ShutdownOutcome::TimedOut`] is a non-terminal observation and is replaced
/// by whatever the next attempt reports.
pub(crate) const fn merge_outcome(
    previous: Option<ShutdownOutcome>,
    observed: ShutdownOutcome,
) -> ShutdownOutcome {
    match previous {
        Some(ShutdownOutcome::Completed) => ShutdownOutcome::Completed,
        Some(ShutdownOutcome::AbnormalTermination) => ShutdownOutcome::AbnormalTermination,
        Some(ShutdownOutcome::TimedOut) | None => observed,
    }
}

/// Choose the bound for the next drain.
///
/// After a timeout the next attempt re-checks with a zero bound instead of
/// blocking again, which is how a repeated call reports the drain's current
/// state without adding another wait to a failing test.
pub(crate) const fn drain_bound(
    previous: Option<ShutdownOutcome>,
    configured: Option<Duration>,
) -> Option<Duration> {
    match previous {
        Some(ShutdownOutcome::TimedOut) => Some(Duration::ZERO),
        _ => configured,
    }
}

#[cfg(test)]
mod tests {
    //! Exhaustive tests over the closed outcome domain.

    use std::sync::Arc;

    use tokio::time::{Duration, timeout};
    use wireframe::server::ServerError;

    use super::{ShutdownOutcome, classify_drain, drain_bound, merge_outcome};
    use crate::TestError;

    /// Every outcome the record can hold, including "nothing observed yet".
    const OUTCOMES: [Option<ShutdownOutcome>; 4] = [
        None,
        Some(ShutdownOutcome::Completed),
        Some(ShutdownOutcome::AbnormalTermination),
        Some(ShutdownOutcome::TimedOut),
    ];

    /// Every outcome a single drain can report.
    const OBSERVED: [ShutdownOutcome; 3] = [
        ShutdownOutcome::Completed,
        ShutdownOutcome::AbnormalTermination,
        ShutdownOutcome::TimedOut,
    ];

    #[test]
    fn merge_outcome_is_monotone() {
        let mut visited = 0_u8;
        for previous in OUTCOMES {
            for observed in OBSERVED {
                visited += 1;
                let merged = merge_outcome(previous, observed);
                match previous {
                    Some(ShutdownOutcome::Completed) => assert_eq!(
                        merged,
                        ShutdownOutcome::Completed,
                        "Completed must absorb {observed:?}"
                    ),
                    Some(ShutdownOutcome::AbnormalTermination) => assert_eq!(
                        merged,
                        ShutdownOutcome::AbnormalTermination,
                        "AbnormalTermination must absorb {observed:?}"
                    ),
                    Some(ShutdownOutcome::TimedOut) | None => assert_eq!(
                        merged, observed,
                        "a non-terminal record must adopt {observed:?}"
                    ),
                }
            }
        }
        assert_eq!(visited, 12, "the enumeration must visit all twelve pairs");
    }

    #[tokio::test]
    async fn classify_drain_distinguishes_every_class() {
        let (clean_outcome, clean_result) = classify_drain(Ok(Ok(())));
        assert_eq!(clean_outcome, ShutdownOutcome::Completed);
        assert!(
            clean_result.is_ok(),
            "a clean drain must classify as success"
        );

        let returned = ServerError::AbnormalTermination {
            message: "returned".to_owned(),
            source: Some(Arc::new(ServerError::AbnormalTermination {
                message: "inner".to_owned(),
                source: None,
            })),
        };
        let (returned_outcome, returned_result) = classify_drain(Ok(Err(returned)));
        assert_eq!(returned_outcome, ShutdownOutcome::AbnormalTermination);
        match returned_result {
            Err(TestError::Server(ServerError::AbnormalTermination { message, .. })) => {
                assert_eq!(message, "returned", "the original error must survive");
            }
            other => panic!("expected the typed server error, got {other:?}"),
        }

        let panicked = ServerError::AbnormalTermination {
            message: "panicked".to_owned(),
            source: None,
        };
        let (panicked_outcome, panicked_result) = classify_drain(Ok(Err(panicked)));
        assert_eq!(panicked_outcome, ShutdownOutcome::AbnormalTermination);
        match panicked_result {
            Err(TestError::Server(ServerError::AbnormalTermination { message, .. })) => {
                assert_eq!(message, "panicked", "the panic diagnostic must survive");
            }
            other => panic!("expected the typed server error, got {other:?}"),
        }

        let elapsed = timeout(Duration::ZERO, std::future::pending::<()>())
            .await
            .expect_err("a zero bound must elapse before pending resolves");
        let (elapsed_outcome, elapsed_result) = classify_drain(Err(elapsed));
        assert_eq!(elapsed_outcome, ShutdownOutcome::TimedOut);
        assert!(
            matches!(elapsed_result, Err(TestError::Timeout(_))),
            "an elapsed bound must classify as a timeout"
        );
    }

    /// Check one previous-outcome and configured-bound pair.
    fn check_drain_bound(previous: Option<ShutdownOutcome>, configured: Option<Duration>) {
        let expected = if previous == Some(ShutdownOutcome::TimedOut) {
            Some(Duration::ZERO)
        } else {
            configured
        };
        assert_eq!(
            drain_bound(previous, configured),
            expected,
            "drain bound for {previous:?} with {configured:?}"
        );
    }

    #[test]
    fn drain_bound_never_blocks_after_timeout() {
        let configured = [None, Some(Duration::from_secs(7))];
        let mut visited = 0_u8;
        for previous in OUTCOMES {
            for bound in configured {
                visited += 1;
                check_drain_bound(previous, bound);
            }
        }
        assert_eq!(visited, 8, "the enumeration must visit all eight cases");
    }
}
