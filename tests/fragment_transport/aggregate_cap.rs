//! Transport regressions for aggregate fragment admission.

use std::{io, num::NonZeroUsize, time::Duration};

use tokio::{sync::mpsc, time::timeout};
use wireframe::{
    app::Envelope,
    fragment::{
        FragmentHeader,
        FragmentIndex,
        FragmentationConfig,
        MessageId,
        encode_fragment_payload,
    },
};

use crate::fragment_helpers::{
    CORRELATION,
    ROUTE_ID,
    TestError,
    TestResult,
    fragmentation_config,
    make_app,
    send_envelopes,
    spawn_app,
};

/// Buffer capacity shared by the flood tests.
const CAPACITY: usize = 512;
/// One-byte partial messages sent before the budget must reject one.
const PARTIAL_COUNT: u64 = 9;
/// Partials the budget must admit before rejecting the excess one.
const ADMITTED_PARTIALS: usize = 8;

#[tokio::test]
async fn unique_partial_flood_aborts_at_concurrent_cap() -> TestResult {
    let mut config = fragmentation_config(CAPACITY)?;
    config.reassembly_timeout = Duration::from_secs(30);
    config.max_concurrent_partials = NonZeroUsize::new(8);
    config.max_buffered_bytes = None;
    assert_partial_flood_aborts(config).await
}

#[tokio::test]
async fn unique_partial_flood_aborts_at_aggregate_byte_cap() -> TestResult {
    let mut config = fragmentation_config(CAPACITY)?;
    config.reassembly_timeout = Duration::from_secs(30);
    config.max_concurrent_partials = None;
    config.max_buffered_bytes = NonZeroUsize::new(8);
    assert_partial_flood_aborts(config).await
}

/// Send one-byte partials for distinct message IDs and require that the
/// fragment exceeding the configured budget aborts the connection with
/// [`io::ErrorKind::InvalidData`] before any partial reaches the handler.
async fn assert_partial_flood_aborts(config: FragmentationConfig) -> TestResult {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(CAPACITY, config, &tx)?;
    let (mut client, mut server) = spawn_app(app);

    let fragments = (0..PARTIAL_COUNT)
        .map(partial_envelope)
        .collect::<TestResult<Vec<_>>>()?;

    let (admitted, excess) = fragments.split_at(ADMITTED_PARTIALS);
    send_envelopes(&mut client, admitted).await?;
    if timeout(Duration::from_millis(100), &mut server)
        .await
        .is_ok()
    {
        return Err(TestError::Assertion(
            "connection closed before the cap".into(),
        ));
    }
    send_envelopes(&mut client, excess).await?;

    let outcome = timeout(Duration::from_secs(5), server).await??;
    match outcome {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {}
        other => {
            return Err(TestError::Assertion(format!(
                "expected InvalidData, got {other:?}"
            )));
        }
    }
    if rx.try_recv().is_ok() {
        return Err(TestError::Assertion(
            "partial message reached handler".into(),
        ));
    }
    Ok(())
}

/// Encode a one-byte non-final fragment envelope for message `id`.
fn partial_envelope(id: u64) -> TestResult<Envelope> {
    let header = FragmentHeader::new(MessageId::new(id), FragmentIndex::zero(), false);
    let payload = encode_fragment_payload(header, &[42])?;
    Ok(Envelope::new(ROUTE_ID, CORRELATION, payload))
}
