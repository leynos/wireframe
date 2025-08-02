//! Tests for [`WireframeServer`] configuration.

use wireframe::{
    app::{Envelope, WireframeApp},
    server::WireframeServer,
};

#[test]
fn default_worker_count_matches_cpu_count() {
    let server = WireframeServer::new(|| {
        WireframeApp::<_, _, Envelope>::new().expect("WireframeApp::new failed")
    });
    let expected = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    assert_eq!(server.worker_count(), expected);
}

#[test]
fn default_workers_at_least_one() {
    let server = WireframeServer::new(|| {
        WireframeApp::<_, _, Envelope>::new().expect("WireframeApp::new failed")
    });
    assert!(server.worker_count() >= 1);
}

#[test]
fn workers_method_enforces_minimum() {
    let server = WireframeServer::new(|| {
        WireframeApp::<_, _, Envelope>::new().expect("WireframeApp::new failed")
    })
    .workers(0);
    assert_eq!(server.worker_count(), 1);
}

#[test]
fn workers_accepts_large_values() {
    let server = WireframeServer::new(|| {
        WireframeApp::<_, _, Envelope>::new().expect("WireframeApp::new failed")
    })
    .workers(128);
    assert_eq!(server.worker_count(), 128);
}

/// Ensure dropping the readiness receiver logs a warning and does not
/// prevent the server from accepting connections.
#[tokio::test]
async fn readiness_receiver_dropped() {
    use tokio::{
        net::TcpStream,
        sync::oneshot,
        time::{Duration, sleep},
    };

    let factory = || WireframeApp::<_, _, Envelope>::new().expect("WireframeApp::new failed");
    let server = WireframeServer::new(factory)
        .workers(1)
        .bind("127.0.0.1:0".parse().unwrap())
        .unwrap();

    let addr = server.local_addr().unwrap();
    // Create channel and immediately drop receiver to force send failure
    let (tx_ready, rx_ready) = oneshot::channel();
    drop(rx_ready);

    tokio::spawn(async move {
        server
            .ready_signal(tx_ready)
            .run_with_shutdown(tokio::time::sleep(Duration::from_millis(200)))
            .await
            .unwrap();
    });

    // Wait briefly to ensure server attempted to send readiness signal
    sleep(Duration::from_millis(100)).await;

    // Server should still accept connections
    let _stream = TcpStream::connect(addr).await.expect("connect failed");
}
