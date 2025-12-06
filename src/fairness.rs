//! Helpers tracking fairness counters for connection processing.
//!
//! This module encapsulates the logic for deciding when high-priority
//! processing should yield to low-priority traffic based on configured
//! thresholds and optional time slices.  A pluggable [`Clock`] allows the
//! timing logic to be tested without depending on Tokio's global time.

use tokio::time::Instant;

use crate::connection::FairnessConfig;

/// Time source used by [`FairnessTracker`].
pub(crate) trait Clock: Clone {
    /// Return the current instant.
    fn now(&self) -> Instant;
}

/// Clock implementation backed by [`tokio::time`].
#[derive(Clone, Debug, Default)]
pub(crate) struct TokioClock;

impl Clock for TokioClock {
    fn now(&self) -> Instant { Instant::now() }
}

#[derive(Debug)]
pub(crate) struct FairnessTracker<C: Clock = TokioClock> {
    config: FairnessConfig,
    clock: C,
    high_counter: usize,
    high_start: Option<Instant>,
}

impl FairnessTracker {
    pub(crate) fn new(config: FairnessConfig) -> Self { Self::with_clock(config, TokioClock) }
}

impl<C: Clock> FairnessTracker<C> {
    pub(crate) fn with_clock(config: FairnessConfig, clock: C) -> Self {
        Self {
            config,
            clock,
            high_counter: 0,
            high_start: None,
        }
    }

    pub(crate) fn set_config(&mut self, config: FairnessConfig) {
        self.config = config;
        self.reset();
    }

    pub(crate) fn record_high_priority(&mut self) {
        self.high_counter += 1;
        if self.high_counter == 1 {
            self.high_start = Some(self.clock.now());
        }
    }

    pub(crate) fn should_yield_to_low_priority(&self) -> bool {
        if self.config.max_high_before_low > 0
            && self.high_counter >= self.config.max_high_before_low
        {
            return true;
        }

        if let (Some(slice), Some(start)) = (self.config.time_slice, self.high_start) {
            return self.clock.now().duration_since(start) >= slice;
        }

        false
    }

    pub(crate) fn reset(&mut self) { self.clear(); }

    fn clear(&mut self) {
        self.high_counter = 0;
        self.high_start = None;
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use std::sync::{Arc, Mutex};

    use rstest::rstest;
    use tokio::time::{Duration, Instant};

    use super::*;

    #[rstest]
    #[case::threshold_2_then_reset(2, 2, true, false)]
    #[case::threshold_1_then_reset(1, 1, true, false)]
    #[test]
    fn fairness_threshold_behaviour(
        #[case] max_high_before_low: usize,
        #[case] calls_before_assert: usize,
        #[case] expected_before_reset: bool,
        #[case] expected_after_reset: bool,
    ) {
        let cfg = FairnessConfig {
            max_high_before_low,
            time_slice: None,
        };
        let mut fairness = FairnessTracker::new(cfg);
        for _ in 0..calls_before_assert {
            fairness.record_high_priority();
        }

        assert_eq!(
            fairness.should_yield_to_low_priority(),
            expected_before_reset
        );

        fairness.reset();
        assert_eq!(
            fairness.should_yield_to_low_priority(),
            expected_after_reset
        );
    }

    #[rstest]
    #[test]
    fn zero_threshold_without_slice_does_not_yield() {
        let cfg = FairnessConfig {
            max_high_before_low: 0,
            time_slice: None,
        };
        let mut fairness = FairnessTracker::new(cfg);
        fairness.record_high_priority();
        assert!(!fairness.should_yield_to_low_priority());
        fairness.record_high_priority();
        assert!(!fairness.should_yield_to_low_priority());
    }

    #[derive(Clone, Debug)]
    struct MockClock {
        now: Arc<Mutex<Instant>>,
    }

    impl MockClock {
        fn new(start: Instant) -> Self {
            Self {
                now: Arc::new(Mutex::new(start)),
            }
        }

        #[expect(clippy::expect_used, reason = "poisoned lock should fail tests loudly")]
        fn advance(&self, dur: Duration) {
            let mut now = self.now.lock().expect("MockClock mutex poisoned");
            *now += dur;
        }
    }

    impl Clock for MockClock {
        #[expect(clippy::expect_used, reason = "poisoned lock should fail tests loudly")]
        fn now(&self) -> Instant { *self.now.lock().expect("MockClock mutex poisoned") }
    }

    #[rstest]
    #[test]
    fn time_slice_triggers_yield() {
        let start = Instant::now();
        let clock = MockClock::new(start);
        let cfg = FairnessConfig {
            max_high_before_low: 0,
            time_slice: Some(Duration::from_millis(5)),
        };
        let mut fairness = FairnessTracker::with_clock(cfg, clock.clone());
        fairness.record_high_priority();
        assert!(!fairness.should_yield_to_low_priority());
        clock.advance(Duration::from_millis(5));
        assert!(fairness.should_yield_to_low_priority());
    }
}
