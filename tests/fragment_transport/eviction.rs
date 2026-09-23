//! Eviction tests for fragment reassembly timeout behaviour.
//!
//! Verifies that incomplete fragment sequences are correctly evicted after
//! the reassembly timeout expires, preventing memory leaks from abandoned
//! message streams.

use std::time::Duration;

use tokio::{
    sync::mpsc,
    time::{sleep, timeout},
};
use wireframe::{app::Envelope, fragment::Fragmenter};

use crate::fragment_helpers::{
    CORRELATION,
    ROUTE_ID,
    TestError,
    TestResult,
    fragment_envelope,
    fragmentation_config_with_timeout,
    make_app,
    send_envelopes,
    spawn_app,
};

#[tokio::test]
async fn expired_fragments_are_evicted() -> TestResult {
    let buffer_capacity = 512;
    let timeout_ms = 10;
    let config = fragmentation_config_with_timeout(buffer_capacity, timeout_ms)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx)?;
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);
    let (mut client, server) = spawn_app(app);

    let payload = vec![3_u8; 800];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload);
    let fragments = fragment_envelope(&request, &fragmenter)?;

    // Send the first fragment then pause long enough for eviction.
    let first_fragment = fragments
        .get(..1)
        .ok_or(TestError::Setup("fragmenter produced no fragments"))?;
    send_envelopes(&mut client, first_fragment).await?;
    sleep(Duration::from_millis(timeout_ms * 2)).await;
    if let Some(rest) = fragments.get(1..) {
        send_envelopes(&mut client, rest).await?;
    }
    tokio::io::AsyncWriteExt::shutdown(client.get_mut()).await?;

    let recv_result = timeout(Duration::from_millis(200), rx.recv()).await;
    if recv_result.is_ok() {
        return Err(TestError::Assertion(format!(
            "handler should not receive after timeout eviction, got {recv_result:?}"
        )));
    }

    drop(client);
    server.await??;

    Ok(())
}
