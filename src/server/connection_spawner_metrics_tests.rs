//! Metric regressions for rejected server connection preambles.

use std::{io, sync::Arc};

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use tokio::net::{TcpListener, TcpStream};

use super::{PreambleFailure, PreambleHandler, run_preamble_failure, run_preamble_success};

#[tokio::test]
async fn preamble_budget_rejections_are_counted_when_handler_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("read test listener address");
    let client = TcpStream::connect(addr).await.expect("connect test client");
    let (mut stream, _) = listener.accept().await.expect("accept test client");
    let handler: PreambleFailure = Arc::new(|_, _| Box::pin(async { Ok(()) }));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let errors = [
        bincode::error::DecodeError::LimitExceeded,
        bincode::error::DecodeError::Other(crate::preamble::PREAMBLE_TOO_LONG_ERROR),
    ];

    ::metrics::with_local_recorder(&recorder, || {
        for error in errors {
            futures::executor::block_on(run_preamble_failure(
                Some(&handler),
                error,
                &mut stream,
                None,
            ));
        }
    });

    let metric_samples = snapshotter.snapshot().into_vec();
    assert!(metric_samples.iter().any(|(key, _, _, value)| {
        key.key().name() == crate::metrics::DECODE_REJECTIONS
            && key
                .key()
                .labels()
                .any(|label| label.key() == "path" && label.value() == "preamble")
            && key
                .key()
                .labels()
                .any(|label| label.key() == "reason" && label.value() == "limit_exceeded")
            && matches!(value, DebugValue::Counter(count) if *count == 2)
    }));
    drop(client);
}

/// A success-handler error increments the post-decode rejection counter.
#[tokio::test]
async fn success_handler_errors_are_counted_as_preamble_rejections() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("read test listener address");
    let client = TcpStream::connect(addr).await.expect("connect test client");
    let (mut stream, _) = listener.accept().await.expect("accept test client");
    let handler: PreambleHandler<()> = Arc::new(|_preamble, _stream| {
        Box::pin(async { Err::<(), io::Error>(io::Error::other("rejected")) })
    });
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    let result = ::metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(run_preamble_success(Some(&handler), &(), &mut stream, None))
    });
    assert!(result.is_err(), "a failing success handler must reject");

    let metric_samples = snapshotter.snapshot().into_vec();
    assert!(metric_samples.iter().any(|(key, _, _, value)| {
        key.key().name() == crate::metrics::PREAMBLE_REJECTIONS
            && key
                .key()
                .labels()
                .any(|label| label.key() == "reason" && label.value() == "success_handler_error")
            && matches!(value, DebugValue::Counter(count) if *count == 1)
    }));
    drop(client);
}
