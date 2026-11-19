//! Regression tests for the read step's shutdown exit.
//!
//! A continuing response leaves a prefetched decoder failure for the read step
//! to apply the recovery policy to. Shutdown can win the next read first, so
//! the step returns `None` while the buffer still holds that failure. These
//! tests drive [`next_frame`] itself rather than the report helper, so
//! removing the report from that branch fails them.

use std::time::Duration;

#[cfg(feature = "metrics")]
use metrics_util::debugging::DebuggingRecorder;
use tokio::io::duplex;
use tokio_util::sync::CancellationToken;

use super::*;
#[cfg(feature = "metrics")]
use crate::app::inbound_handler::recovery::test_support::policy_count;
use crate::{
    app::{
        inbound_handler::recovery::test_support::{RecordingHook, protocol_failure, tight_config},
        recovery_codec::DecodedItem,
    },
    codec::RecoveryPolicy,
    hooks::ProtocolHooks,
    test_helpers::TestCodec,
};

/// Read once with `server_shutdown` already cancelled.
///
/// The connection never receives bytes: this exercises only the exit that
/// returns `None` while a decoder failure is still buffered. The runtime's own
/// token is a child of `server_shutdown`, so cancelling the server token ends
/// the read exactly as a real shutdown does.
async fn read_after_shutdown(hook: &RecordingHook, server_shutdown: &CancellationToken) {
    let codec = TestCodec::new(64);
    let (client, server) = duplex(64);
    let (sink, mut reader, connection_codec) = prepare_connection_framing(server, &codec).split();
    let limits = resolve_framing_limits(None, &connection_codec);
    let mut inbound =
        prepare_inbound_state::<TestCodec>(None, ProtocolHooks::default(), None, limits);
    // The failure a continuing response left behind for this read.
    inbound.pending = Some(DecodedItem::Failure(protocol_failure()));

    let mut runtime = ConnectionRuntime::prepare::<TestCodec>(RuntimeParams {
        protocol: None,
        server_shutdown,
        recovery_hook: Some(hook),
        recovery_config: tight_config(),
        peer_address: None,
    })
    .expect("the connection runtime builds for the test codec");

    server_shutdown.cancel();

    let poll = ReadPoll {
        read_timeout_ms: 50,
        shutdown: server_shutdown,
        budgets: limits.effective_budgets,
    };
    let read = next_frame::<_, TestCodec>(&mut inbound, &mut reader, &mut runtime, poll)
        .await
        .expect("a cancelled read ends without an error");

    assert!(read.is_none(), "shutdown ends the read");
    assert!(
        inbound.pending.is_none(),
        "the read step must report the failure it can no longer act on"
    );
    // Keep the transport halves alive until the read has finished, so no EOF
    // can be mistaken for the shutdown that ended it.
    drop((client, sink));
}

#[tokio::test]
async fn shutdown_ended_read_reports_the_buffered_failure() {
    // The hook would have dropped this failure, but the connection is already
    // ending, so reporting a drop would describe an action that never ran.
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let shutdown = CancellationToken::new();

    read_after_shutdown(&hook, &shutdown).await;

    assert_eq!(hook.error_types(), vec!["protocol"]);
    assert_eq!(
        hook.applied_policies(),
        vec![RecoveryPolicy::Disconnect],
        "the hook must observe the disconnect that actually happened"
    );
}

#[cfg(feature = "metrics")]
#[test]
fn shutdown_ended_read_records_a_disconnect_metric() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    // The recorder is thread-local, so the future that records must run on
    // this thread; a current-thread runtime keeps it there.
    metrics::with_local_recorder(&recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime builds for the metric test");
        runtime.block_on(async {
            let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
            let shutdown = CancellationToken::new();
            read_after_shutdown(&hook, &shutdown).await;
        });
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(policy_count(&samples, "disconnect"), 1);
    assert_eq!(
        policy_count(&samples, "drop"),
        0,
        "a policy that was never applied must not be recorded"
    );
}
