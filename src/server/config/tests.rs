//! Tests for server configuration utilities.
//!
//! This module exercises the `WireframeServer` builder, covering worker counts,
//! binding behaviour, preamble handling, callback registration, and method
//! chaining. Fixtures from `test_util` provide shared setup and parameterised
//! cases via `rstest`.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use rstest::rstest;

use super::*;
use crate::server::test_util::{
    TestPreamble,
    bind_server,
    factory,
    free_listener,
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
    free_listener: std::net::TcpListener,
) {
    let addr = free_listener
        .local_addr()
        .expect("failed to read listener address");
    drop(free_listener);
    let local_addr = WireframeServer::new(factory)
        .bind(addr)
        .expect("Failed to bind")
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr.ip(), addr.ip());
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
    let local_addr = bind_server(factory, free_listener).local_addr().unwrap();
    assert_eq!(
        local_addr.ip(),
        std::net::IpAddr::from(std::net::Ipv4Addr::LOCALHOST)
    );
}

#[rstest]
#[case("success")]
#[case("failure")]
#[tokio::test]
async fn test_preamble_callback_registration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] callback_type: &str,
) {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();

    let server = server_with_preamble(factory);
    let server = match callback_type {
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
        _ => panic!("Invalid callback type"),
    };

    assert_eq!(counter.load(Ordering::SeqCst), 0);
    match callback_type {
        "success" => assert!(server.on_preamble_success.is_some()),
        "failure" => assert!(server.on_preamble_failure.is_some()),
        _ => unreachable!(),
    }
}

#[rstest]
#[tokio::test]
async fn test_method_chaining(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let callback_invoked = Arc::new(AtomicUsize::new(0));
    let counter = callback_invoked.clone();
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
        .bind_listener(free_listener)
        .expect("Failed to bind");
    assert_eq!(server.worker_count(), 2);
    assert!(server.local_addr().is_some());
    assert_eq!(callback_invoked.load(Ordering::SeqCst), 0);
}

#[rstest]
#[tokio::test]
async fn test_server_configuration_persistence(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let server = WireframeServer::new(factory)
        .workers(5)
        .bind_listener(free_listener)
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
    free_listener: std::net::TcpListener,
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
        .bind_listener(free_listener)
        .expect("Failed to bind first address");
    let first = server.local_addr().expect("first bound address missing");
    let server = server.bind(addr2).expect("Failed to bind second address");
    let second = server.local_addr().expect("second bound address missing");
    assert_ne!(first.port(), second.port());
    assert_eq!(second.ip(), addr2.ip());
}
