//! Tests for [`WireframeServer`] configuration.

use wireframe::{app::WireframeApp, server::WireframeServer};

#[test]
fn default_worker_count_matches_cpu_count() {
    let server = WireframeServer::new(|| WireframeApp::new().expect("WireframeApp::new failed"));
    let expected = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    assert_eq!(server.worker_count(), expected);
}

#[test]
fn default_workers_at_least_one() {
    let server = WireframeServer::new(|| WireframeApp::new().expect("WireframeApp::new failed"));
    assert!(server.worker_count() >= 1);
}

#[test]
fn workers_method_enforces_minimum() {
    let server =
        WireframeServer::new(|| WireframeApp::new().expect("WireframeApp::new failed")).workers(0);
    assert_eq!(server.worker_count(), 1);
}

#[test]
fn workers_accepts_large_values() {
    let server = WireframeServer::new(|| WireframeApp::new().expect("WireframeApp::new failed"))
        .workers(128);
    assert_eq!(server.worker_count(), 128);
}
