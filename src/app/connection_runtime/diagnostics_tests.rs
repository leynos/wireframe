//! Bounded diagnostics for connection panic and teardown reporting.
//!
//! These tests pin the ordering the diagnostics exist for: a processing panic
//! is reported before a teardown that might never complete, and a teardown
//! panic is reported once without being retried.

use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::FutureExt;
use tokio::{
    io::{AsyncWriteExt, duplex},
    runtime::Builder,
    sync::Notify,
    time::timeout,
};
use tracing_test::traced_test;

use super::tests::{drive, request_frame};
use crate::{
    app::{Envelope, Handler, WireframeApp},
    serializer::BincodeSerializer,
};

/// Handles observed by a gated teardown callback.
struct TeardownProbe {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    completions: Arc<AtomicUsize>,
    panic_seen: Arc<AtomicBool>,
}

/// Build an app whose handler panics and whose teardown waits for the test.
///
/// The teardown callback samples the tracing capture at entry, before it
/// waits. A diagnostic recorded there is proof that the panic was reported
/// before teardown had a chance to complete. The sampler is supplied by the
/// caller because `logs_contain` is injected into the traced test body.
fn panicking_app_with_gated_teardown<F>(
    probe: TeardownProbe,
    diagnostic_visible: F,
) -> WireframeApp<BincodeSerializer, (), Envelope>
where
    F: Fn() -> bool + Send + Sync + 'static,
{
    let TeardownProbe {
        entered,
        release,
        completions,
        panic_seen,
    } = probe;
    let handler: Handler<Envelope> =
        Arc::new(|_: &Envelope| Box::pin(async { panic!("handler boom") }));
    let sampler = Arc::new(diagnostic_visible);
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build application")
        .route(1, handler)
        .expect("register route")
        .on_connection_setup(|| async {})
        .expect("register setup hook")
        .on_connection_teardown(move |()| {
            let teardown_entered = Arc::clone(&entered);
            let teardown_release = Arc::clone(&release);
            let teardown_completions = Arc::clone(&completions);
            let teardown_panic_seen = Arc::clone(&panic_seen);
            let teardown_sampler = Arc::clone(&sampler);
            Box::pin(async move {
                teardown_panic_seen.store(teardown_sampler(), Ordering::SeqCst);
                teardown_entered.notify_one();
                teardown_release.notified().await;
                teardown_completions.fetch_add(1, Ordering::SeqCst);
            })
        })
        .expect("register teardown hook")
}

/// A handler panic is reported before a pending teardown completes.
///
/// The teardown callback records whether the panic diagnostic already exists
/// when it starts, then waits for the test to release it. That proves the
/// diagnostic does not depend on teardown finishing.

#[traced_test]
#[test]
fn handler_panic_is_logged_before_pending_teardown_completes() {
    let probe = TeardownProbe {
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
        completions: Arc::new(AtomicUsize::new(0)),
        panic_seen: Arc::new(AtomicBool::new(false)),
    };
    let entered = Arc::clone(&probe.entered);
    let release = Arc::clone(&probe.release);
    let completions = Arc::clone(&probe.completions);
    let panic_seen = Arc::clone(&probe.panic_seen);

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let prepared = runtime
        .block_on(
            panicking_app_with_gated_teardown(probe, || {
                logs_contain("connection processing panicked")
            })
            .prepare(),
        )
        .expect("prepare application");

    runtime.block_on(async {
        let (mut client, server) = duplex(64);
        let connection = tokio::spawn(async move {
            AssertUnwindSafe(drive(&prepared, server, None))
                .catch_unwind()
                .await
        });
        client
            .write_all(&request_frame(1))
            .await
            .expect("write request frame");
        client.shutdown().await.expect("close client");

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("teardown should start");
        assert!(
            panic_seen.load(Ordering::SeqCst),
            "the panic diagnostic should be emitted before teardown completes"
        );
        assert_eq!(
            completions.load(Ordering::SeqCst),
            0,
            "teardown should still be pending"
        );

        release.notify_one();
        let panic = connection
            .await
            .expect("join connection task")
            .expect_err("handler panic should propagate");
        assert_eq!(panic.downcast_ref::<&str>().copied(), Some("handler boom"));
        assert_eq!(
            completions.load(Ordering::SeqCst),
            1,
            "teardown should run exactly once"
        );
    });
}

/// A teardown panic is logged once, is not retried, and still propagates.
#[traced_test]
#[test]
fn teardown_panic_is_logged_once_and_propagates() {
    let calls = Arc::new(AtomicUsize::new(0));
    let teardown_calls = Arc::clone(&calls);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let prepared = runtime
        .block_on(
            WireframeApp::<BincodeSerializer, (), Envelope>::new()
                .expect("build application")
                .on_connection_setup(|| async {})
                .expect("register setup hook")
                .on_connection_teardown(move |()| {
                    let teardown_counter = Arc::clone(&teardown_calls);
                    Box::pin(async move {
                        teardown_counter.fetch_add(1, Ordering::SeqCst);
                        panic!("teardown boom");
                    })
                })
                .expect("register teardown hook")
                .prepare(),
        )
        .expect("prepare application");

    let panic = runtime
        .block_on(async {
            let (client, server) = duplex(64);
            drop(client);
            AssertUnwindSafe(drive(&prepared, server, None))
                .catch_unwind()
                .await
        })
        .expect_err("teardown panic should propagate");

    assert_eq!(panic.downcast_ref::<&str>().copied(), Some("teardown boom"));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "teardown should run exactly once"
    );
    logs_assert(|lines: &[&str]| {
        let matching = lines
            .iter()
            .filter(|line| line.contains("connection teardown panicked"))
            .count();
        match matching {
            1 => Ok(()),
            other => Err(format!(
                "expected exactly one teardown-failure log, found {other}"
            )),
        }
    });
}
