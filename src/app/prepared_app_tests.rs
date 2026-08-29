//! Tests for prepared-application instrumentation seams.

use std::{cell::Cell, time::Duration};

use super::*;

/// Deterministic preparation time source that records the calls it serves.
struct FixedPreparationTimeSource {
    starts: Cell<usize>,
    elapsed: Cell<usize>,
}

impl PreparationTimeSource for FixedPreparationTimeSource {
    type StartedAt = ();

    fn start(&self) -> Self::StartedAt { self.starts.set(self.starts.get() + 1); }

    fn elapsed(&self, _started_at: Self::StartedAt) -> Duration {
        self.elapsed.set(self.elapsed.get() + 1);
        Duration::from_millis(1)
    }
}

/// Deterministic connection time source that records the calls it serves.
struct FixedConnectionTimeSource {
    starts: Cell<usize>,
    elapsed: Cell<usize>,
}

impl ConnectionTimeSource for FixedConnectionTimeSource {
    type StartedAt = ();

    fn start(&self) -> Self::StartedAt { self.starts.set(self.starts.get() + 1); }

    fn elapsed(&self, _started_at: Self::StartedAt) -> Duration {
        self.elapsed.set(self.elapsed.get() + 1);
        Duration::from_millis(1)
    }
}

/// Preparation records timing through the injected source exactly once.
#[tokio::test]
async fn prepare_uses_injected_time_source() {
    let app: WireframeApp = WireframeApp::new().expect("app should initialize");
    let time_source = FixedPreparationTimeSource {
        starts: Cell::new(0),
        elapsed: Cell::new(0),
    };

    let _prepared = app
        .prepare_with_time_source(&time_source)
        .await
        .expect("preparation should succeed");

    assert_eq!(time_source.starts.get(), 1);
    assert_eq!(time_source.elapsed.get(), 1);
}

/// Prepared connection tracing records timing through the injected source.
#[tokio::test]
async fn prepared_connection_uses_injected_time_source() {
    let prepared: PreparedApp = WireframeApp::new()
        .expect("app should initialize")
        .prepare()
        .await
        .expect("preparation should succeed");
    let time_source = FixedConnectionTimeSource {
        starts: Cell::new(0),
        elapsed: Cell::new(0),
    };
    let (client, server) = tokio::io::duplex(64);
    drop(client);

    prepared
        .handle_connection_with_time_source(server, &time_source)
        .await
        .expect("clean EOF should complete");

    assert_eq!(time_source.starts.get(), 1);
    assert_eq!(time_source.elapsed.get(), 1);
}
