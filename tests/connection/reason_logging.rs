//! Tests for the logged reason when multi-packet handling closes.

use super::*;

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
