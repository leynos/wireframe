//! Tests for server configuration utilities.
//!
//! This module exercises the `WireframeServer` builder, covering worker counts,
//! binding behaviour, preamble handling, handler registration, and method
//! chaining. Fixtures from `test_util` provide shared setup and parameterised
//! cases via `rstest`.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::rstest;

use super::*;
use crate::server::test_util::{
    TestPreamble,
    bind_server,
    factory,
    free_port,
    server_with_preamble,
};

fn expected_default_worker_count() -> usize {
    // Mirror the default worker logic to keep tests aligned with `WireframeServer::new`.
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

#[rstest]
fn test_new_server_creation(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    assert!(server.worker_count() >= 1 && server.local_addr().is_none());
}

#[rstest]
fn test_new_server_default_worker_count(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    assert_eq!(server.worker_count(), expected_default_worker_count());
}

#[rstest]
fn test_workers_configuration(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let mut server = WireframeServer::new(factory);
    server = server.workers(4);
    assert_eq!(server.worker_count(), 4);
    server = server.workers(100);
    assert_eq!(server.worker_count(), 100);
    assert_eq!(server.workers(0).worker_count(), 1);
}

#[rstest]
fn test_with_preamble_type_conversion(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory).with_preamble::<TestPreamble>();
    assert_eq!(server.worker_count(), expected_default_worker_count());
}

#[rstest]
#[tokio::test]
async fn test_bind_success(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let local_addr = WireframeServer::new(factory)
        .bind(free_port)
        .expect("Failed to bind")
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr.ip(), free_port.ip());
}

#[rstest]
fn test_local_addr_before_bind(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    assert!(WireframeServer::new(factory).local_addr().is_none());
}

#[rstest]
#[tokio::test]
async fn test_local_addr_after_bind(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let local_addr = bind_server(factory, free_port).local_addr().unwrap();
    assert_eq!(local_addr.ip(), free_port.ip());
}

#[rstest]
#[case("success")]
#[case("failure")]
#[tokio::test]
async fn test_preamble_handler_registration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] handler_type: &str,
) {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();

    let server = server_with_preamble(factory);
    let server = match handler_type {
        "success" => server.on_preamble_decode_success(move |_p: &TestPreamble, _| {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
        "failure" => server.on_preamble_decode_failure(move |_err: &DecodeError| {
            c.fetch_add(1, Ordering::SeqCst);
        }),
        _ => panic!("Invalid handler type"),
    };

    assert_eq!(counter.load(Ordering::SeqCst), 0);
    match handler_type {
        "success" => assert!(server.on_preamble_success.is_some()),
        "failure" => assert!(server.on_preamble_failure.is_some()),
        _ => unreachable!(),
    }
}

#[rstest]
#[tokio::test]
async fn test_method_chaining(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let handler_invoked = Arc::new(AtomicUsize::new(0));
    let counter = handler_invoked.clone();
    let server = WireframeServer::new(factory)
        .workers(2)
        .with_preamble::<TestPreamble>()
        .on_preamble_decode_success(move |_p: &TestPreamble, _| {
            let c = counter.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
        .on_preamble_decode_failure(|_: &DecodeError| {})
        .bind(free_port)
        .expect("Failed to bind");
    assert_eq!(server.worker_count(), 2);
    assert!(server.local_addr().is_some());
    assert_eq!(handler_invoked.load(Ordering::SeqCst), 0);
}

#[rstest]
#[tokio::test]
async fn test_server_configuration_persistence(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = WireframeServer::new(factory)
        .workers(5)
        .bind(free_port)
        .expect("Failed to bind");
    assert_eq!(server.worker_count(), 5);
    assert!(server.local_addr().is_some());
}

#[rstest]
fn test_extreme_worker_counts(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let mut server = WireframeServer::new(factory);
    server = server.workers(usize::MAX);
    assert_eq!(server.worker_count(), usize::MAX);
    assert_eq!(server.workers(0).worker_count(), 1);
}

#[rstest]
#[tokio::test]
async fn test_bind_to_multiple_addresses(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let listener2 =
        std::net::TcpListener::bind(SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), 0))
            .expect("failed to bind second listener");
    let addr2 = listener2
        .local_addr()
        .expect("failed to get second listener address");
    drop(listener2);

    let server = WireframeServer::new(factory);
    let server = server
        .bind(free_port)
        .expect("Failed to bind first address");
    let first = server.local_addr().expect("first bound address missing");
    let server = server.bind(addr2).expect("Failed to bind second address");
    let second = server.local_addr().expect("second bound address missing");
    assert_ne!(first.port(), second.port());
    assert_eq!(second.ip(), addr2.ip());
}

#[rstest]
fn test_accept_backoff_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let initial = Duration::from_millis(5);
    let max = Duration::from_millis(500);
    let server = WireframeServer::new(factory).accept_backoff(initial, max);
    assert_eq!(server.backoff_config.initial_delay, initial);
    assert_eq!(server.backoff_config.max_delay, max);
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
    let server = WireframeServer::new(factory).accept_initial_delay(delay);
    assert_eq!(server.backoff_config.initial_delay, delay);
}

#[rstest]
fn test_accept_max_delay_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let delay = Duration::from_millis(2000);
    let server = WireframeServer::new(factory).accept_max_delay(delay);
    assert_eq!(server.backoff_config.max_delay, delay);
}

#[rstest]
fn test_backoff_validation(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory.clone()).accept_initial_delay(Duration::ZERO);
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );

    let server = WireframeServer::new(factory)
        .accept_initial_delay(Duration::from_millis(100))
        .accept_max_delay(Duration::from_millis(50));
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
    let server = WireframeServer::new(factory).accept_initial_delay(Duration::from_secs(2));
    assert_eq!(server.backoff_config.initial_delay, Duration::from_secs(2));
    assert_eq!(server.backoff_config.max_delay, Duration::from_secs(2));
}

#[rstest]
fn test_accept_backoff_parameter_swapping(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory.clone())
        .accept_backoff(Duration::from_millis(5), Duration::from_millis(1));
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );
    assert_eq!(server.backoff_config.max_delay, Duration::from_millis(5));

    let server = WireframeServer::new(factory).accept_backoff(Duration::ZERO, Duration::ZERO);
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );
    assert_eq!(server.backoff_config.max_delay, Duration::from_millis(1));
}
