//! Closure-reason logging tests for multi-packet queue drains and shutdown.

use log::Level;
use rstest::rstest;
use serial_test::serial;
use tokio::sync::mpsc;
use wireframe_testing::{LoggerHandle, TestResult, logger};

use crate::support::{HarnessConfig, HarnessFactory, assert_reason_logged, harness_factory};

#[rstest]
#[serial(connection_logs)]
fn handle_multi_packet_closed_logs_reason(
    harness_factory: HarnessFactory,
    mut logger: LoggerHandle,
) -> TestResult {
    logger.clear();
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (_tx, rx) = mpsc::channel(1);
    harness
        .actor_mut()
        .set_multi_packet_with_correlation(Some(rx), Some(11))?;
    logger.clear();
    harness.handle_multi_packet_closed();
    assert_reason_logged(&mut logger, Level::Info, "drained", Some(11));
    Ok(())
}

#[rstest]
#[serial(connection_logs)]
fn try_opportunistic_drain_multi_disconnect_logs_reason(
    harness_factory: HarnessFactory,
    mut logger: LoggerHandle,
) -> TestResult {
    logger.clear();
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (tx, rx) = mpsc::channel(1);
    harness
        .actor_mut()
        .set_multi_packet_with_correlation(Some(rx), Some(12))?;
    drop(tx);
    logger.clear();
    let drained = harness.try_drain_multi();
    if drained {
        return Err("disconnect should not report a drained frame".into());
    }
    assert_reason_logged(&mut logger, Level::Warn, "disconnected", Some(12));
    Ok(())
}

#[rstest]
#[serial(connection_logs)]
fn start_shutdown_logs_reason(
    harness_factory: HarnessFactory,
    mut logger: LoggerHandle,
) -> TestResult {
    logger.clear();
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (_tx, rx) = mpsc::channel(1);
    harness
        .actor_mut()
        .set_multi_packet_with_correlation(Some(rx), Some(13))?;
    logger.clear();
    harness.start_shutdown();
    assert_reason_logged(&mut logger, Level::Info, "shutdown", Some(13));
    if harness.has_multi_queue() {
        return Err("multi-packet queue should be cleared after shutdown".into());
    }
    Ok(())
}
