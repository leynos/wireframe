//! Coverage for connection task spawning and panic recovery behaviour.

use rstest::rstest;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::task::TaskTracker;
use tracing_test::traced_test;

use super::*;
use crate::{app::WireframeApp, server::test_util::factory};

/// Panic payload shapes a connection handler can raise.
///
/// Kept as an enum rather than a closure so the setup callback stays `Fn`
/// and each case selects a payload the logger must render differently.
#[derive(Clone, Copy, Debug)]
enum PanicPayload {
    /// A string message, logged verbatim.
    Message,
    /// A non-string payload, logged with its `Debug` representation.
    NonString,
}

impl PanicPayload {
    fn raise(self) -> ! {
        match self {
            Self::Message => panic!("boom"),
            Self::NonString => std::panic::panic_any(5_u32),
        }
    }
}

/// Panics in connection handlers are logged and do not tear down the worker.
///
/// `#[traced_test]` must precede `#[rstest]`: parameterised cases are emitted
/// into a child module, and the injected `logs_assert` helper is only in scope
/// for the function body it is expanded into.
#[traced_test]
#[rstest]
#[case::string_payload(PanicPayload::Message, "panic=boom")]
#[case::non_string_payload(PanicPayload::NonString, "panic=Any")]
#[tokio::test]
async fn spawn_connection_task_logs_panic(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] payload: PanicPayload,
    #[case] expected_panic: &str,
) {
    let app = Arc::new(
        factory()
            .on_connection_setup(move || async move { payload.raise() })
            .expect("failed to install panic setup callback")
            .prepare()
            .await
            .expect("prepare app"),
    );
    let tracker = TaskTracker::new();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener.local_addr");

    let handle = tokio::spawn({
        let task_tracker = tracker.clone();
        let app = Arc::clone(&app);
        async move {
            let (stream, _) = listener.accept().await.expect("accept");
            spawn_connection_task(stream, app, PreambleHooks::<()>::default(), &task_tracker);
            task_tracker.close();
            task_tracker.wait().await;
        }
    });

    let client = TcpStream::connect(addr).await.expect("connect");
    let peer_addr = client.local_addr().expect("client.local_addr");
    client.writable().await.expect("client.writable");
    client.try_write(&[0; 8]).expect("client.try_write");
    drop(client);

    handle.await.expect("join connection task driver");
    tokio::task::yield_now().await;

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| {
                line.contains("connection task panicked")
                    && line.contains(expected_panic)
                    && line.contains(&format!("peer_addr=Some({peer_addr})"))
            })
            .map(|_| ())
            .ok_or_else(|| format!("panic log not found for {expected_panic}"))
    });
}
