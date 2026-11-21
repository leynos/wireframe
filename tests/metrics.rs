//! Tests for `wireframe` metrics helpers.
//!
//! These tests verify that counters and gauges update as expected using
//! `metrics_util::debugging::DebuggingRecorder`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use rstest::rstest;

/// Creates a debugging recorder and snapshotter for metrics testing.
fn debugging_recorder_setup() -> (Snapshotter, DebuggingRecorder) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    (snapshotter, recorder)
}

#[test]
fn outbound_frame_metric_increments() {
    let (snapshotter, recorder) = debugging_recorder_setup();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_frames(wireframe::metrics::Direction::Outbound);
    });

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics.iter().any(|(k, _, _, v)| {
        k.key().name() == wireframe::metrics::FRAMES_PROCESSED
            && k.key()
                .labels()
                .any(|l| l.key() == "direction" && l.value() == "outbound")
            && matches!(v, DebugValue::Counter(c) if *c > 0)
    });
    assert!(found, "outbound frames metric not recorded");
}

#[test]
fn inbound_frame_metric_increments() {
    let (snapshotter, recorder) = debugging_recorder_setup();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_frames(wireframe::metrics::Direction::Inbound);
    });
    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics.iter().any(|(k, _, _, v)| {
        k.key().name() == wireframe::metrics::FRAMES_PROCESSED
            && k.key()
                .labels()
                .any(|l| l.key() == "direction" && l.value() == "inbound")
            && matches!(v, DebugValue::Counter(c) if *c > 0)
    });

    assert!(found, "inbound frames metric not recorded");
}

#[test]
fn error_metric_increments() {
    let (snapshotter, recorder) = debugging_recorder_setup();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_deser_errors();
    });

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics.iter().any(|(k, _, _, v)| {
        k.key().name() == wireframe::metrics::ERRORS_TOTAL
            && matches!(v, DebugValue::Counter(c) if *c > 0)
    });
    assert!(found, "error metric not recorded");
}

#[rstest]
#[case(1)]
#[case(2)]
fn inc_connection_panics_counts(#[case] expected: u64) {
    // Arrange
    let (snapshotter, recorder) = debugging_recorder_setup();

    // Act
    metrics::with_local_recorder(&recorder, || {
        (0..expected).for_each(|_| wireframe::metrics::inc_connection_panics());
    });

    // Assert
    assert_counter_eq(
        &snapshotter,
        wireframe::metrics::CONNECTION_PANICS,
        expected,
    );
}

fn assert_counter_eq(snapshotter: &Snapshotter, name: &str, expected: u64) {
    let metrics = snapshotter.snapshot().into_vec();
    assert!(
        metrics.iter().any(|(key, _, _, value)| {
            key.key().name() == name && matches!(value, DebugValue::Counter(c) if *c == expected)
        }),
        "expected {name} == {expected}, got {metrics:#?}"
    );
}

// Fallible assertions keep the actor tests' `Result` returns meaningful: an
// assertion failure is reported through the test's error channel instead of
// panicking inside a function that already returns `Result`.
#[cfg(feature = "metrics")]
#[path = "common/fallible_assertions/check.rs"]
mod fallible_check;

#[cfg(feature = "metrics")]
#[path = "common/fallible_assertions/check_eq.rs"]
mod fallible_check_eq;

#[cfg(feature = "metrics")]
mod connection_and_handler_metrics {
    //! Lifecycle assertions for metrics owned by the connection actor.
    //!
    //! [`metrics_util::debugging::Snapshotter::snapshot`] swaps every gauge
    //! and counter back to zero, so each test takes exactly one snapshot at
    //! the lifecycle point it asserts. A second snapshot would report only the
    //! change since the first, and a gauge that was incremented and later
    //! decremented would no longer prove the increment happened.

    use std::{
        future::Future,
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use futures::stream;
    use metrics_util::{
        CompositeKey,
        debugging::{DebugValue, DebuggingRecorder},
    };
    use rstest::{fixture, rstest};
    use tokio_util::sync::CancellationToken;
    use wireframe::{
        NoProtocolError,
        connection::{ConnectionActor, ConnectionChannels},
        hooks::{ConnectionContext, ProtocolHooks},
        metrics::{CONNECTIONS_ACTIVE, ERRORS_TOTAL},
        push::{PushConfigError, PushHandle, PushQueues},
        response::WireframeError,
    };
    use wireframe_testing::TestResult;

    use crate::{fallible_check::check, fallible_check_eq::check_eq};

    /// One entry from a drained [`DebuggingRecorder`] snapshot.
    type MetricEntry = (
        CompositeKey,
        Option<metrics::Unit>,
        Option<metrics::SharedString>,
        DebugValue,
    );

    /// Protocol error used to drive the handler-error counter.
    #[derive(Debug)]
    enum TestError {
        /// The response stream failed while handling a request.
        Kaboom,
    }

    /// Build the push queues and handle shared by the actor tests.
    #[fixture]
    fn push_queues() -> Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError> {
        PushQueues::<u8>::builder()
            .high_capacity(8)
            .low_capacity(8)
            .build()
    }

    /// Run `body` on a current-thread runtime with `recorder` installed.
    ///
    /// A current-thread runtime keeps the actor's metric emissions on the
    /// thread that installed the local recorder; a multi-thread runtime could
    /// schedule them on a thread the recorder cannot see.
    fn run_in_recorder<T>(
        recorder: &DebuggingRecorder,
        body: impl Future<Output = T>,
    ) -> io::Result<T> {
        metrics::with_local_recorder(recorder, || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            Ok(runtime.block_on(body))
        })
    }

    /// Return the gauge recorded under `name` in a drained snapshot.
    fn gauge_value(entries: &[MetricEntry], name: &str) -> Option<f64> {
        entries.iter().find_map(|(key, _, _, value)| {
            if key.key().name() != name {
                return None;
            }
            match value {
                DebugValue::Gauge(reading) => Some(**reading),
                _ => None,
            }
        })
    }

    /// Return the counter recorded under `name` and one label pair.
    fn counter_value(
        entries: &[MetricEntry],
        name: &str,
        label_key: &str,
        label_value: &str,
    ) -> Option<u64> {
        entries.iter().find_map(|(key, _, _, value)| {
            let has_label = key
                .key()
                .labels()
                .any(|label| label.key() == label_key && label.value() == label_value);
            if key.key().name() != name || !has_label {
                return None;
            }
            match value {
                DebugValue::Counter(count) => Some(*count),
                _ => None,
            }
        })
    }

    /// Report an error unless the gauge `name` holds `expected`.
    ///
    /// The values under test are small whole numbers, so comparing their bit
    /// patterns is exact and avoids Clippy's float-comparison lint.
    fn check_gauge(entries: &[MetricEntry], name: &str, expected: f64) -> Result<(), String> {
        let actual = gauge_value(entries, name);
        check(
            actual.map(f64::to_bits) == Some(expected.to_bits()),
            format!("expected gauge {name} == {expected}, got {actual:?} in {entries:#?}"),
        )
    }

    /// Constructing an actor registers one active connection.
    #[rstest]
    fn constructing_an_actor_increments_the_active_connection_gauge(
        push_queues: Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError>,
    ) -> TestResult {
        let (queues, handle) = push_queues?;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let entries = run_in_recorder(&recorder, async move {
            let actor: ConnectionActor<u8, NoProtocolError> =
                ConnectionActor::new(queues, handle, None, CancellationToken::new());
            // Take the only snapshot while the actor still owns its guard.
            let snapshot = snapshotter.snapshot().into_vec();
            drop(actor);
            snapshot
        })?;

        check_gauge(&entries, CONNECTIONS_ACTIVE, 1.0)?;
        Ok(())
    }

    /// Dropping an unrun actor releases its active connection.
    #[rstest]
    fn dropping_an_unrun_actor_decrements_the_active_connection_gauge(
        push_queues: Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError>,
    ) -> TestResult {
        let (queues, handle) = push_queues?;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let entries = run_in_recorder(&recorder, async move {
            let actor: ConnectionActor<u8, NoProtocolError> =
                ConnectionActor::new(queues, handle, None, CancellationToken::new());
            drop(actor);
            snapshotter.snapshot().into_vec()
        })?;

        check_gauge(&entries, CONNECTIONS_ACTIVE, 0.0)?;
        Ok(())
    }

    /// A run driven to completion releases the active connection exactly once.
    #[rstest]
    fn completed_run_releases_the_active_connection_gauge(
        push_queues: Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError>,
    ) -> TestResult {
        let (queues, handle) = push_queues?;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let entries = run_in_recorder(&recorder, async move {
            let response = stream::iter(vec![Ok(1u8)]);
            let mut actor: ConnectionActor<u8, NoProtocolError> = ConnectionActor::new(
                queues,
                handle,
                Some(Box::pin(response)),
                CancellationToken::new(),
            );
            let mut out = Vec::new();
            actor
                .run(&mut out)
                .await
                .map_err(|error| format!("actor run failed: {error:?}"))?;
            check_eq(
                out.as_slice(),
                &[1u8][..],
                "the response frame must be emitted",
            )?;
            // The actor still exists, but `run` released its connection guard.
            let snapshot = snapshotter.snapshot().into_vec();
            drop(actor);
            Ok::<_, String>(snapshot)
        })??;

        check_gauge(&entries, CONNECTIONS_ACTIVE, 0.0)?;
        check_eq(
            counter_value(&entries, ERRORS_TOTAL, "kind", "handler"),
            None,
            "a successful run must not record a handler error",
        )?;
        Ok(())
    }

    /// A protocol error from the response stream records a handler error.
    #[rstest]
    fn protocol_error_increments_the_handler_error_counter(
        push_queues: Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError>,
    ) -> TestResult {
        let (queues, handle) = push_queues?;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let saw_kaboom = Arc::new(AtomicBool::new(false));
        let hook_flag = Arc::clone(&saw_kaboom);

        let (out, entries) = run_in_recorder(&recorder, async move {
            let response = stream::iter(vec![
                Ok(1u8),
                Ok(2u8),
                Err(WireframeError::Protocol(TestError::Kaboom)),
            ]);
            let hooks = ProtocolHooks {
                handle_error: Some(Box::new(
                    move |error: TestError, _ctx: &mut ConnectionContext| {
                        if matches!(error, TestError::Kaboom) {
                            hook_flag.store(true, Ordering::SeqCst);
                        }
                    },
                )),
                ..ProtocolHooks::<u8, TestError>::default()
            };
            let mut actor: ConnectionActor<u8, TestError> = ConnectionActor::with_hooks(
                ConnectionChannels::new(queues, handle),
                Some(Box::pin(response)),
                CancellationToken::new(),
                hooks,
            );
            let mut out = Vec::new();
            actor
                .run(&mut out)
                .await
                .map_err(|error| format!("actor run failed: {error:?}"))?;
            let snapshot = snapshotter.snapshot().into_vec();
            Ok::<_, String>((out, snapshot))
        })??;

        check(
            saw_kaboom.load(Ordering::SeqCst),
            "the handler hook must observe the protocol error",
        )?;
        check_eq(
            out.as_slice(),
            &[1u8, 2u8][..],
            "frames emitted before the protocol error must still be delivered",
        )?;
        check_eq(
            counter_value(&entries, ERRORS_TOTAL, "kind", "handler"),
            Some(1),
            "the protocol error must be counted exactly once",
        )?;
        check_eq(
            counter_value(&entries, ERRORS_TOTAL, "kind", "deserialization"),
            None,
            "a handler error must not be attributed to deserialization",
        )?;
        Ok(())
    }
}
