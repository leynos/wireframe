//! Helpers tracking fairness counters for connection processing.
//!
//! This module encapsulates the logic for deciding when high-priority
//! processing should yield to low-priority traffic based on configured
//! thresholds and optional time slices. A pluggable clock decouples the
//! implementation from the runtime and allows deterministic tests.

use std::time::Duration;

use crate::connection::FairnessConfig;

/// Abstraction over a time source returning [`Instant`]s.
pub(crate) trait Clock {
    type Instant: Copy;
    fn now(&self) -> Self::Instant;
    fn elapsed(&self, start: Self::Instant) -> Duration;
}

#[derive(Debug, Default)]
pub(crate) struct TokioClock;

impl Clock for TokioClock {
    type Instant = tokio::time::Instant;
    fn now(&self) -> Self::Instant {
        tokio::time::Instant::now()
    }
    fn elapsed(&self, start: Self::Instant) -> Duration {
        start.elapsed()
    }
}

#[derive(Debug)]
pub(crate) struct FairnessTracker<C: Clock = TokioClock> {
    config: FairnessConfig,
    high_counter: usize,
    high_start: Option<C::Instant>,
    clock: C,
}

impl FairnessTracker<TokioClock> {
    pub(crate) fn new(config: FairnessConfig) -> Self {
        Self::with_clock(config, TokioClock)
    }
}

impl<C: Clock> FairnessTracker<C> {
    pub(crate) fn with_clock(config: FairnessConfig, clock: C) -> Self {
        Self {
            config,
            high_counter: 0,
            high_start: None,
            clock,
        }
    }

    pub(crate) fn set_config(&mut self, config: FairnessConfig) {
        self.config = config;
        self.clear();
    }

    pub(crate) fn after_high(&mut self) {
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
            if self.clock.elapsed(start) >= slice {
                return true;
            }
        }

        false
    }

    pub(crate) fn after_low(&mut self) {
        self.clear();
    }

    pub(crate) fn reset(&mut self) {
        self.clear();
    }

    fn clear(&mut self) {
        self.high_counter = 0;
        self.high_start = None;
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tokio::time::{self, Duration};

    use super::*;

    #[rstest]
    #[tokio::test]
    async fn yield_after_threshold() {
        let cfg = FairnessConfig {
            max_high_before_low: 2,
            time_slice: None,
        };
        let mut fairness = FairnessTracker::new(cfg);
        fairness.after_high();
        assert!(!fairness.should_yield_to_low_priority());
        fairness.after_high();
        assert!(fairness.should_yield_to_low_priority());
    }

    #[rstest]
    #[tokio::test]
    async fn after_low_resets_counter() {
        let cfg = FairnessConfig {
            max_high_before_low: 1,
            time_slice: None,
        };
        let mut fairness = FairnessTracker::new(cfg);
        fairness.after_high();
        assert!(fairness.should_yield_to_low_priority());
        fairness.after_low();
        assert!(!fairness.should_yield_to_low_priority());
    }

    #[rstest]
    #[tokio::test]
    async fn time_slice_triggers_yield() {
        time::pause();
        let cfg = FairnessConfig {
            max_high_before_low: 0,
            time_slice: Some(Duration::from_millis(5)),
        };
        let mut fairness = FairnessTracker::new(cfg);
        fairness.after_high();
        time::advance(Duration::from_millis(6)).await;
        assert!(fairness.should_yield_to_low_priority());
    }
}
