//! Process-isolated coverage for the parked-body gauge.
//!
//! `wireframe_streaming_bodies_parked` is a process-wide gauge, so a sibling
//! test that parks a message can move the value between the read and the
//! assertion, and a handler that panics on purpose writes `panicked at` into
//! whatever log the gate is collecting. Both problems disappear when each
//! ending is driven in its own process: the child installs the metrics
//! recorder, drives one ending on a production connection over an in-memory
//! transport, and prints the value it read at each step; the parent runs the
//! children and reads those values back.
//!
//! The three endings are the ones a connection can reach: the loop returning
//! because the peer stopped, the connection task being cancelled while a
//! message is parked, and a streaming handler unwinding through the loop. Each
//! child asserts its own numbers, so the parent only has to report a child that
//! fails and refuse one that claims to have run nothing.

use std::{io, process::Command, sync::Arc};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tokio::{sync::Notify, task::JoinHandle};
use wireframe_testing::{TestError, TestResult};

use super::watch_support::{
    App,
    Client,
    STEP_WAIT,
    Scenario,
    bounded_for,
    ensure,
    open_body,
    read_reply,
    send_complete,
    send_request,
    spawn_connection,
};
use crate::{
    app::{Envelope, ResponseHandler, StreamingHandler, WireframeApp},
    metrics::STREAMING_BODIES_PARKED,
    request::{RequestBodyStream, RequestParts},
    response::Response,
    test_helpers::{TestAssembler, continuation_frame_payload},
};

/// Route whose streaming handler owns the dispatch slot.
const LIVE_ROUTE: u32 = 80;
/// Buffered route whose completed messages are parked behind the live body.
const PARKED_ROUTE: u32 = 81;
/// Message key of the live streaming body.
const LIVE_KEY: u64 = 61;
/// Message key of the first parked message.
const FIRST_KEY: u64 = 62;
/// Message key of the second parked message.
const SECOND_KEY: u64 = 63;
/// Payload that marks the parked message whose reply is held back.
const GATED_PAYLOAD: &[u8] = b"first";
/// Payload of the parked message answered without a gate.
const SECOND_PAYLOAD: &[u8] = b"second";
/// Message key of the sentinel connection's live body.
const SENTINEL_LIVE_KEY: u64 = 64;
/// Message key of the message parked on the sentinel connection.
const SENTINEL_PARKED_KEY: u64 = 65;

/// One ending this harness drives in a child process.
#[derive(Clone, Copy, Debug)]
enum Ending {
    /// The peer stops and the loop returns with the parked queue drained.
    NormalExit,
    /// The connection task is cancelled while a message is parked.
    TaskAbort,
    /// A streaming handler unwinds through the connection loop.
    HandlerPanic,
}

impl Ending {
    /// Every ending the parent requires evidence for.
    const ALL: [Self; 3] = [Self::NormalExit, Self::TaskAbort, Self::HandlerPanic];

    /// Name of the child test that drives this ending.
    const fn child(self) -> &'static str {
        match self {
            Self::NormalExit => "child_normal_exit",
            Self::TaskAbort => "child_task_abort",
            Self::HandlerPanic => "child_handler_panic",
        }
    }

    /// Value the child prints once it has driven this ending.
    const fn label(self) -> &'static str {
        match self {
            Self::NormalExit => "normal_exit",
            Self::TaskAbort => "task_abort",
            Self::HandlerPanic => "handler_panic",
        }
    }
}

/// Drive every ending in its own process and require each to report back.
///
/// A child that makes no assertions still has to print its completion marker,
/// so a filter that matches no test cannot pass this: the marker is the proof
/// that the scenario ran rather than that the process exited.
#[test]
fn the_parked_gauge_returns_to_baseline_for_every_ending() {
    for ending in Ending::ALL {
        let name = child_test_name(ending.child());
        let output = Command::new(std::env::current_exe().expect("the test binary path is known"))
            .args([
                "--ignored",
                "--exact",
                &name,
                "--test-threads=1",
                // The child's report is what proves it ran, and its expected
                // panic output must reach this process rather than a log.
                "--nocapture",
            ])
            .output()
            .expect("the child test process starts");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "the {} ending failed in its child process:\n{stdout}\n{stderr}",
            ending.label()
        );
        assert!(
            stdout.contains(&format!("scenario={}", ending.label())),
            "the {} ending did not run: the child reported no scenario marker\n{stdout}\n{stderr}",
            ending.label()
        );
    }
}

/// Full libtest name of one child test in this module.
///
/// `module_path!()` is prefixed with the crate name, which libtest does not
/// include in the names it filters on.
fn child_test_name(child: &str) -> String {
    let module = scenarios::MODULE;
    let relative = module.split_once("::").map_or(module, |(_, rest)| rest);
    format!("{relative}::{child}")
}

/// Install the metrics recorder this process reports to.
///
/// A child process runs exactly one ending, so the global recorder is free.
fn install_recorder() -> TestResult<Snapshotter> {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    recorder
        .install()
        .map_err(|_| TestError::Msg("the metrics recorder is installed once per process".into()))?;
    Ok(snapshotter)
}

/// Current value of the parked-body gauge.
///
/// The recorder keeps one sample per series, so the last publication is the
/// current value; a series that has never been published reads as zero.
fn parked_gauge(snapshotter: &Snapshotter) -> f64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| key.key().name() == STREAMING_BODIES_PARKED)
        .filter_map(|(_, _, _, value)| match value {
            DebugValue::Gauge(total) => Some(total.into_inner()),
            _ => None,
        })
        .next_back()
        .unwrap_or(0.0)
}

/// Whether the parked-body gauge reads exactly `expected` messages.
///
/// The gauge carries a count of messages, so every published value is a whole
/// number that `f64` represents exactly; a tolerance would accept a value the
/// connection never published.
#[expect(
    clippy::float_cmp,
    reason = "the gauge carries whole message counts, which f64 represents exactly"
)]
fn gauge_reads(snapshotter: &Snapshotter, expected: f64) -> bool {
    parked_gauge(snapshotter) == expected
}

/// Wait until the parked-body gauge reads `expected`.
///
/// The connection runs on its own task, so the publication that follows a park
/// is observed by yielding until it lands rather than by sleeping.
async fn await_gauge(snapshotter: &Snapshotter, expected: f64) -> TestResult<()> {
    for _ in 0..STEP_WAIT.as_millis() {
        if gauge_reads(snapshotter, expected) {
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
    Err(TestError::Msg(format!(
        "the parked gauge never read {expected}; it reads {}",
        parked_gauge(snapshotter)
    )))
}

#[cfg(test)]
#[path = "stream_parked_gauge_support.rs"]
mod support;

#[cfg(test)]
#[path = "stream_parked_gauge_scenarios.rs"]
mod scenarios;
