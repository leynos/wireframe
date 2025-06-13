use wireframe::{app::WireframeApp, server::WireframeServer};

#[test]
fn default_worker_count_is_positive() {
    let server = WireframeServer::new(|| WireframeApp::new().unwrap());
    assert!(server.worker_count() >= 1);
}

#[test]
fn workers_method_enforces_minimum() {
    let server = WireframeServer::new(|| WireframeApp::new().unwrap()).workers(0);
    assert_eq!(server.worker_count(), 1);
}
