//! Tests for server configuration utilities.
//!
//! This module exercises the `WireframeServer` builder, covering worker counts,
//! binding behaviour, preamble handling, handler registration, and method
//! chaining. Fixtures from `test_util` provide shared setup and parameterised
//! cases via `rstest`.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bincode::error::DecodeError;
use rstest::{fixture, rstest};
use tokio::net::{TcpListener, TcpStream};

use super::*;
use crate::server::{
    test_util::{
        TestPreamble,
        bind_server,
        factory,
        free_listener,
        listener_addr,
        server_with_preamble,
    },
    BackoffConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreambleHandlerKind {
    Success,
    Failure,
}

fn expected_default_worker_count() -> usize {
    // Mirror the default worker logic to keep tests aligned with `WireframeServer::new`.
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

#[fixture]
async fn connected_streams() -> io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let client = TcpStream::connect(addr).await?;
    let (server, _) = listener.accept().await?;
    Ok((client, server))
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
    let server = WireframeServer::new(factory);
    let server = server.workers(4);
    assert_eq!(server.worker_count(), 4);
    let server = server.workers(100);
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
fn test_preamble_timeout_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let timeout = Duration::from_millis(25);
    let server = WireframeServer::new(factory).preamble_timeout(timeout);
    assert_eq!(server.preamble_timeout, Some(timeout));

    let clamped = WireframeServer::new(factory).preamble_timeout(Duration::ZERO);
    assert_eq!(clamped.preamble_timeout, Some(Duration::from_millis(1)));
}

#[rstest]
#[tokio::test]
async fn test_bind_success(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let expected = listener_addr(&free_listener);
    let local_addr = WireframeServer::new(factory)
        .bind_existing_listener(free_listener)
        .expect("Failed to bind")
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr, expected);
}

#[rstest]
fn test_local_addr_before_bind(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    assert!(WireframeServer::new(factory).local_addr().is_none());
}

#[rstest]
#[tokio::test]
async fn test_local_addr_after_bind(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let expected = listener_addr(&free_listener);
    let local_addr = bind_server(factory, free_listener)
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr, expected);
}

#[rstest]
#[case::success(PreambleHandlerKind::Success)]
#[case::failure(PreambleHandlerKind::Failure)]
#[tokio::test]
async fn test_preamble_handler_registration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] handler: PreambleHandlerKind,
    connected_streams: io::Result<(TcpStream, TcpStream)>,
) -> io::Result<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();

    let server = server_with_preamble(factory);
    let server = match handler {
        PreambleHandlerKind::Success => server.on_preamble_decode_success(move |_p: &TestPreamble, _| {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
        PreambleHandlerKind::Failure => server.on_preamble_decode_failure(
            move |_err: &DecodeError, _stream| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), io::Error>(())
                })
            },
        ),
    };

    assert_eq!(counter.load(Ordering::SeqCst), 0);
    match handler {
        PreambleHandlerKind::Success => {
            assert!(server.on_preamble_success.is_some());
            let handler = server
                .on_preamble_success
                .as_ref()
                .expect("success handler missing");
            let (_client, mut stream) = connected_streams?;
            let preamble = TestPreamble { id: 0, message: String::new() };
            handler(&preamble, &mut stream).await?;
        }
        PreambleHandlerKind::Failure => {
            assert!(server.on_preamble_failure.is_some());
            let handler = server
                .on_preamble_failure
                .as_ref()
                .expect("failure handler missing");
            let (_client, mut stream) = connected_streams?;
            handler(&DecodeError::UnexpectedEnd, &mut stream).await?;
        }
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_method_chaining(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
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
        .on_preamble_decode_failure(|_: &DecodeError, _| Box::pin(async { Ok::<(), io::Error>(()) }))
        .bind_existing_listener(free_listener)
        .expect("Failed to bind");
    assert_eq!(server.worker_count(), 2);
    assert!(server.local_addr().is_some());
    assert_eq!(handler_invoked.load(Ordering::SeqCst), 0);
}

#[rstest]
#[tokio::test]
async fn test_server_configuration_persistence(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let server = WireframeServer::new(factory)
        .workers(5)
        .bind_existing_listener(free_listener)
        .expect("Failed to bind");
    assert_eq!(server.worker_count(), 5);
    assert!(server.local_addr().is_some());
}

#[rstest]
fn test_extreme_worker_counts(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    let server = server.workers(usize::MAX);
    assert_eq!(server.worker_count(), usize::MAX);
    assert_eq!(server.workers(0).worker_count(), 1);
}

#[rstest]
#[tokio::test]
async fn test_bind_to_multiple_addresses(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let addr1 = listener_addr(&free_listener);

    let server = WireframeServer::new(factory);
    let server = server
        .bind_existing_listener(free_listener)
        .expect("Failed to bind first address");
    let first = server.local_addr().expect("first bound address missing");
    assert_eq!(first, addr1);

    let server = server
        .bind(std::net::SocketAddr::new(addr1.ip(), 0))
        .expect("Failed to bind second address");
    let second = server.local_addr().expect("second bound address missing");
    assert_eq!(second.ip(), addr1.ip());
    assert_ne!(first.port(), second.port());
}

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
    let initial = Duration::from_millis(10);
    let max = Duration::from_millis(80);
    let attempts = 5;

    let sequence = backoff_sequence(initial, max, attempts);

    let expected_delays = [
        initial,
        std::cmp::min(initial.saturating_mul(2), max),
        std::cmp::min(initial.saturating_mul(4), max),
        std::cmp::min(initial.saturating_mul(8), max),
        max,
    ];

    assert_eq!(&sequence[..], &expected_delays);
}

fn backoff_sequence(initial: Duration, max: Duration, attempts: usize) -> Vec<Duration> {
    let mut sequence = Vec::with_capacity(attempts);
    let mut backoff = initial;

    for _ in 0..attempts {
        sequence.push(backoff);
        backoff = std::cmp::min(backoff * 2, max);
    }

    sequence
}

#[rstest]
fn test_accept_initial_delay_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let delay = Duration::from_millis(20);
    let cfg = BackoffConfig { initial_delay: delay, ..BackoffConfig::default() };
    let server = WireframeServer::new(factory).accept_backoff(cfg);
    assert_eq!(server.backoff_config.initial_delay, delay);
}

#[rstest]
fn test_accept_max_delay_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let delay = Duration::from_millis(2000);
    let cfg = BackoffConfig { max_delay: delay, ..BackoffConfig::default() };
    let server = WireframeServer::new(factory).accept_backoff(cfg);
    assert_eq!(server.backoff_config.max_delay, delay);
}

#[rstest]
fn test_backoff_validation(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory.clone())
        .accept_backoff(BackoffConfig { initial_delay: Duration::ZERO, ..BackoffConfig::default() });
    assert_eq!(
        server.backoff_config.initial_delay,
        Duration::from_millis(1)
    );

    let server = WireframeServer::new(factory)
        .accept_backoff(BackoffConfig {
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
