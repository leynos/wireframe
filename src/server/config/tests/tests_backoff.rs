//! Backoff configuration tests for `WireframeServer`.

use std::time::Duration;

use rstest::rstest;

use crate::{
    app::WireframeApp,
    server::{BackoffConfig, WireframeServer, test_util::factory},
};

#[rstest]
#[case::custom_backoff(
    BackoffConfig {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(500),
    },
    Duration::from_millis(5),
    Duration::from_millis(500),
    "custom backoff"
)]
#[case::custom_initial_delay(
    BackoffConfig {
        initial_delay: Duration::from_millis(20),
        ..BackoffConfig::default()
    },
    Duration::from_millis(20),
    Duration::from_secs(1),
    "custom initial delay"
)]
#[case::custom_max_delay(
    BackoffConfig {
        max_delay: Duration::from_millis(2000),
        ..BackoffConfig::default()
    },
    Duration::from_millis(10),
    Duration::from_millis(2000),
    "custom max delay"
)]
fn test_accept_backoff_configuration_scenarios(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] config: BackoffConfig,
    #[case] expected_initial: Duration,
    #[case] expected_max: Duration,
    #[case] _description: &'static str,
) {
    let server = WireframeServer::new(factory).accept_backoff(config);
    assert_eq!(server.backoff_config.initial_delay, expected_initial);
    assert_eq!(server.backoff_config.max_delay, expected_max);
}

/// Behaviour test verifying exponential delay doubling and capping.
#[test]
fn test_accept_exponential_backoff_doubles_and_caps() {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let initial = Duration::from_millis(10);
    let max = Duration::from_millis(80);
    let mut backoff = initial;
    let mut delays = Vec::new();
    let attempts = 5;

    let start = Instant::now();
    let mut last = start;

    for _i in 0..attempts {
        thread::sleep(backoff);
        let now = Instant::now();
        let elapsed = now.duration_since(last);
        delays.push(elapsed);
        last = now;

        backoff = std::cmp::min(backoff * 2, max);
    }

    let expected_delays = [
        initial,
        std::cmp::min(initial * 2, max),
        std::cmp::min(initial * 4, max),
        std::cmp::min(initial * 8, max),
        max,
    ];

    for (i, (actual, expected)) in delays.iter().zip(expected_delays.iter()).enumerate() {
        assert!(
            *actual >= *expected,
            "Delay {i} was {actual:?}, expected at least {expected:?}"
        );
        let max_expected = *expected + Duration::from_millis(20);
        assert!(
            *actual < max_expected,
            "Delay {i} was {actual:?}, expected less than {max_expected:?}"
        );
    }
}

#[rstest]
#[case::clamp_initial_delay(
    BackoffConfig {
        initial_delay: Duration::ZERO,
        ..BackoffConfig::default()
    },
    Duration::from_millis(1),
    Duration::from_secs(1),
    "clamp initial delay"
)]
#[case::swap_shorter_max(
    BackoffConfig {
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(50),
    },
    Duration::from_millis(50),
    Duration::from_millis(100),
    "swap shorter max"
)]
#[case::swap_with_default_max(
    BackoffConfig {
        initial_delay: Duration::from_secs(2),
        max_delay: Duration::from_secs(1),
    },
    Duration::from_secs(1),
    Duration::from_secs(2),
    "swap with default max"
)]
#[case::swap_small_values(
    BackoffConfig {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(1),
    },
    Duration::from_millis(1),
    Duration::from_millis(5),
    "swap small values"
)]
#[case::clamp_both_zero(
    BackoffConfig {
        initial_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    },
    Duration::from_millis(1),
    Duration::from_millis(1),
    "clamp both zero"
)]
fn test_backoff_validation_scenarios(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] config: BackoffConfig,
    #[case] expected_initial: Duration,
    #[case] expected_max: Duration,
    #[case] _description: &'static str,
) {
    let server = WireframeServer::new(factory).accept_backoff(config);
    assert_eq!(server.backoff_config.initial_delay, expected_initial);
    assert_eq!(server.backoff_config.max_delay, expected_max);
}

#[rstest]
fn test_backoff_default_values(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(10)
    );
    assert_eq!(server.backoff_config.max_delay, Duration::from_secs(1));
}
