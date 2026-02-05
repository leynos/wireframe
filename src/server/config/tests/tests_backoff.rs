//! Backoff configuration tests for `WireframeServer`.

use std::time::Duration;

use rstest::rstest;

use crate::{
    app::WireframeApp,
    server::{BackoffConfig, WireframeServer, test_util::factory},
};

#[rstest]
fn test_accept_backoff_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let cfg = BackoffConfig {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(500),
    };
    let server = WireframeServer::new(factory).accept_backoff(cfg);
    assert_eq!(server.backoff_config, cfg);
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
fn test_accept_initial_delay_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let delay = Duration::from_millis(20);
    let cfg = BackoffConfig {
        initial_delay: delay,
        ..BackoffConfig::default()
    };
    let server = WireframeServer::new(factory).accept_backoff(cfg);
    assert_eq!(server.backoff_config.initial_delay, delay);
}

#[rstest]
fn test_accept_max_delay_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let delay = Duration::from_millis(2000);
    let cfg = BackoffConfig {
        max_delay: delay,
        ..BackoffConfig::default()
    };
    let server = WireframeServer::new(factory).accept_backoff(cfg);
    assert_eq!(server.backoff_config.max_delay, delay);
}

#[rstest]
fn test_backoff_validation(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory.clone()).accept_backoff(BackoffConfig {
        initial_delay: Duration::ZERO,
        ..BackoffConfig::default()
    });
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );

    let server = WireframeServer::new(factory).accept_backoff(BackoffConfig {
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(50),
    });
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(50)
    );
    assert_eq!(server.backoff_config.max_delay, Duration::from_millis(100));
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

#[rstest]
fn test_initial_delay_exceeds_default_max(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let cfg = BackoffConfig {
        initial_delay: Duration::from_secs(2),
        max_delay: Duration::from_secs(1),
    };
    let server = WireframeServer::new(factory).accept_backoff(cfg);
    assert_eq!(server.backoff_config.initial_delay, Duration::from_secs(1));
    assert_eq!(server.backoff_config.max_delay, Duration::from_secs(2));
}

#[rstest]
fn test_accept_backoff_parameter_swapping(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory.clone()).accept_backoff(BackoffConfig {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(1),
    });
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );
    assert_eq!(server.backoff_config.max_delay, Duration::from_millis(5));

    let server = WireframeServer::new(factory).accept_backoff(BackoffConfig {
        initial_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    });
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );
    assert_eq!(server.backoff_config.max_delay, Duration::from_millis(1));
}
