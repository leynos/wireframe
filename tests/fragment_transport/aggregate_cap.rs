//! Transport regression for aggregate fragment admission.

use std::{io, num::NonZeroUsize, time::Duration};

use tokio::{sync::mpsc, time::timeout};
use wireframe::{
    app::Envelope,
    fragment::{FragmentHeader, FragmentIndex, MessageId, encode_fragment_payload},
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

#[tokio::test]
async fn unique_partial_flood_aborts_at_concurrent_cap() -> TestResult {
    let capacity = 512;
    let mut config = fragmentation_config(capacity)?;
    config.reassembly_timeout = Duration::from_secs(30);
    config.max_concurrent_partials = NonZeroUsize::new(8);
    config.max_buffered_bytes = None;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(capacity, config, &tx)?;
    let (mut client, mut server) = spawn_app(app);
    let mut fragments = Vec::new();
    for id in 0..9 {
        let header = FragmentHeader::new(MessageId::new(id), FragmentIndex::zero(), false);
        let payload = encode_fragment_payload(header, &[42])?;
        fragments.push(Envelope::new(ROUTE_ID, CORRELATION, payload));
    }
    let admitted = fragments
        .get(..8)
        .ok_or(TestError::Setup("missing admitted fragments"))?;
    send_envelopes(&mut client, admitted).await?;
    if timeout(Duration::from_millis(100), &mut server)
        .await
        .is_ok()
    {
        return Err(TestError::Assertion(
            "connection closed before the cap".into(),
        ));
    }
    let excess = fragments
        .get(8..)
        .ok_or(TestError::Setup("missing excess fragment"))?;
    send_envelopes(&mut client, excess).await?;
    let result = timeout(Duration::from_secs(5), server).await??;
    match result {
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
