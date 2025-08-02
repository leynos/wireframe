//! Helpers tracking fairness counters for connection processing.
//!
//! This module encapsulates the logic for deciding when high-priority
//! processing should yield to low-priority traffic based on configured
//! thresholds and optional time slices.

use tokio::time::Instant;

use crate::connection::FairnessConfig;

#[derive(Debug)]
pub(crate) struct Fairness {
    config: FairnessConfig,
    high_counter: usize,
    high_start: Option<Instant>,
}

impl Fairness {
    pub(crate) fn new(config: FairnessConfig) -> Self {
        Self {
            config,
            high_counter: 0,
            high_start: None,
        }
    }

    pub(crate) fn set_config(&mut self, config: FairnessConfig) {
        self.config = config;
        self.reset();
    }

    pub(crate) fn after_high(&mut self) {
        self.high_counter += 1;
        if self.high_counter == 1 {
            self.high_start = Some(Instant::now());
        }
    }

    pub(crate) fn should_yield(&self) -> bool {
        let threshold_hit = self.config.max_high_before_low > 0
            && self.high_counter >= self.config.max_high_before_low;
        let time_hit = self
            .config
            .time_slice
            .zip(self.high_start)
            .is_some_and(|(slice, start)| start.elapsed() >= slice);
        threshold_hit || time_hit
    }

    pub(crate) fn after_low(&mut self) { self.reset(); }

    pub(crate) fn reset(&mut self) {
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
        let mut fairness = Fairness::new(cfg);
        fairness.after_high();
        assert!(!fairness.should_yield());
        fairness.after_high();
        assert!(fairness.should_yield());
    }

    #[rstest]
    #[tokio::test]
    async fn after_low_resets_counter() {
        let cfg = FairnessConfig {
            max_high_before_low: 1,
            time_slice: None,
        };
        let mut fairness = Fairness::new(cfg);
        fairness.after_high();
        assert!(fairness.should_yield());
        fairness.after_low();
        assert!(!fairness.should_yield());
    }

    #[rstest]
    #[tokio::test]
    async fn time_slice_triggers_yield() {
        time::pause();
        let cfg = FairnessConfig {
            max_high_before_low: 0,
            time_slice: Some(Duration::from_millis(5)),
        };
        let mut fairness = Fairness::new(cfg);
        fairness.after_high();
        time::advance(Duration::from_millis(6)).await;
        assert!(fairness.should_yield());
    }
}
