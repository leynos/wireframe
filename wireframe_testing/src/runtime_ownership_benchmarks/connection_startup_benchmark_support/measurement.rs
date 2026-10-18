//! Driving one connection-startup workload.
//!
//! A connection is a `tokio::io::duplex` pair rather than a socket, so the
//! measured cost sits in application preparation and per-connection setup
//! instead of the kernel loopback path. The client half is dropped before the
//! connection is served, which turns it into a clean immediate end-of-stream:
//! no frame round-trip is timed here.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use tokio::io::duplex;
use wireframe::test_helpers::connection_startup::{
    ConnectionStartupCounts,
    ConnectionStartupInstrumentation,
    StartupTestApp,
    counted_app_factory,
};

use super::workload::{StartupMeasurement, StartupPath, StartupWorkload};

/// Capacity of each in-memory duplex pipe, in bytes.
const DUPLEX_CAPACITY: usize = 64;

/// Serve one connection from an already-prepared application.
async fn serve_connection(prepared: &wireframe::app::PreparedApp) -> Result<(), String> {
    let (client, server) = duplex(DUPLEX_CAPACITY);
    // Dropping the client half before serving turns the connection into a clean
    // immediate end-of-stream, so no frame handling is timed here.
    drop(client);
    prepared
        .handle_connection_result(server)
        .await
        .map_err(|err| format!("prepared connection failed: {err}"))
}

/// Build and prepare a fresh application, serve one connection, then discard it.
async fn serve_via_per_connection_factory(
    instrumentation: &ConnectionStartupInstrumentation,
    workload: StartupWorkload,
) -> Result<(), String> {
    let factory = counted_app_factory(instrumentation.clone(), workload.plan());
    let app: StartupTestApp = factory().map_err(|err| format!("factory failed: {err}"))?;
    let prepared = app
        .prepare()
        .await
        .map_err(|err| format!("preparation failed: {err}"))?;
    serve_connection(&prepared).await
}

/// Prepare once and serve every connection from the shared root.
async fn serve_via_prepared_root(
    instrumentation: &ConnectionStartupInstrumentation,
    workload: StartupWorkload,
) -> Result<(), String> {
    let factory = counted_app_factory(instrumentation.clone(), workload.plan());
    let app: StartupTestApp = factory().map_err(|err| format!("factory failed: {err}"))?;
    let prepared = app
        .prepare()
        .await
        .map_err(|err| format!("preparation failed: {err}"))?;
    for _ in 0..workload.connections {
        serve_connection(&prepared).await?;
    }
    Ok(())
}

/// Run one iteration and report the counters it recorded.
async fn startup_once(
    workload: StartupWorkload,
    instrumentation: &ConnectionStartupInstrumentation,
) -> Result<ConnectionStartupCounts, String> {
    match workload.path {
        StartupPath::PerConnectionFactory => {
            for _ in 0..workload.connections {
                serve_via_per_connection_factory(instrumentation, workload).await?;
            }
        }
        StartupPath::PreparedRoot => {
            serve_via_prepared_root(instrumentation, workload).await?;
        }
    }
    Ok(black_box(instrumentation.snapshot()))
}

/// Attempt `iterations` startup workloads, reporting the last iteration's counts.
///
/// # Errors
///
/// Returns a description of the invalid workload, factory failure, preparation
/// failure, or connection failure.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::runtime_ownership_benchmarks::{
///     StartupPath,
///     StartupWorkload,
///     measure_startup,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_startup(
///     StartupWorkload {
///         path: StartupPath::PreparedRoot,
///         routes: 2,
///         middleware_layers: 2,
///         connections: 4,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.factory_calls, 1);
/// assert_eq!(measurement.transforms, 4);
/// # Ok(())
/// # }
/// ```
pub async fn measure_startup(
    workload: StartupWorkload,
    iterations: u64,
) -> Result<StartupMeasurement, String> {
    workload.validate()?;
    if iterations == 0 {
        return Ok(StartupMeasurement {
            operations: 0,
            connections: workload.connections,
            factory_calls: 0,
            transforms: 0,
            elapsed: Duration::ZERO,
        });
    }

    // Counters are created inside the timed region so the recorded figures
    // describe this measurement and not any earlier warm-up.
    let started = Instant::now();
    let mut counts = ConnectionStartupCounts::default();
    for _ in 0..iterations {
        let instrumentation = ConnectionStartupInstrumentation::new();
        counts = startup_once(workload, &instrumentation).await?;
    }
    let elapsed = started.elapsed();

    Ok(StartupMeasurement {
        operations: iterations,
        connections: workload.connections,
        factory_calls: counts.factory_calls,
        transforms: counts.transforms,
        elapsed,
    })
}
