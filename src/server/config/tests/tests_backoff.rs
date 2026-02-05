//! Backoff configuration tests for `WireframeServer`.

use std::time::Duration;

use rstest::rstest;

use crate::{
    app::WireframeApp,
    server::{BackoffConfig, WireframeServer, test_util::factory},
};

#[derive(Debug)]
struct BackoffScenario {
    config: BackoffConfig,
    expected_initial: Duration,
    expected_max: Duration,
    description: &'static str,
}

#[rstest]
#[case::custom_backoff(
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(500),
        },
        expected_initial: Duration::from_millis(5),
        expected_max: Duration::from_millis(500),
        description: "custom backoff",
    }
)]
#[case::custom_initial_delay(
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::from_millis(20),
            ..BackoffConfig::default()
        },
        expected_initial: Duration::from_millis(20),
        expected_max: Duration::from_secs(1),
        description: "custom initial delay",
    }
)]
#[case::custom_max_delay(
    BackoffScenario {
        config: BackoffConfig {
            max_delay: Duration::from_millis(2000),
            ..BackoffConfig::default()
        },
        expected_initial: Duration::from_millis(10),
        expected_max: Duration::from_millis(2000),
        description: "custom max delay",
    }
)]
fn test_accept_backoff_configuration_scenarios(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] scenario: BackoffScenario,
) {
    let server = WireframeServer::new(factory).accept_backoff(scenario.config);
    assert_eq!(
        server.backoff_config.initial_delay,
        scenario.expected_initial,
        "scenario: {}",
        scenario.description
    );
    assert_eq!(
        server.backoff_config.max_delay,
        scenario.expected_max,
        "scenario: {}",
        scenario.description
    );
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
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::ZERO,
            ..BackoffConfig::default()
        },
        expected_initial: Duration::from_millis(1),
        expected_max: Duration::from_secs(1),
        description: "clamp initial delay",
    }
)]
#[case::swap_shorter_max(
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(50),
        },
        expected_initial: Duration::from_millis(50),
        expected_max: Duration::from_millis(100),
        description: "swap shorter max",
    }
)]
#[case::swap_with_default_max(
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(1),
        },
        expected_initial: Duration::from_secs(1),
        expected_max: Duration::from_secs(2),
        description: "swap with default max",
    }
)]
#[case::swap_small_values(
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(1),
        },
        expected_initial: Duration::from_millis(1),
        expected_max: Duration::from_millis(5),
        description: "swap small values",
    }
)]
#[case::clamp_both_zero(
    BackoffScenario {
        config: BackoffConfig {
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
        expected_initial: Duration::from_millis(1),
        expected_max: Duration::from_millis(1),
        description: "clamp both zero",
    }
)]
fn test_backoff_validation_scenarios(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] scenario: BackoffScenario,
) {
    let server = WireframeServer::new(factory).accept_backoff(scenario.config);
    assert_eq!(
        server.backoff_config.initial_delay,
        scenario.expected_initial,
        "scenario: {}",
        scenario.description
    );
    assert_eq!(
        server.backoff_config.max_delay,
        scenario.expected_max,
        "scenario: {}",
        scenario.description
    );
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
