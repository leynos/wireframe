//! Metric regressions for rejected server connection preambles.

use std::sync::Arc;

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use tokio::net::{TcpListener, TcpStream};

use super::{PreambleFailure, run_preamble_failure};

#[tokio::test]
async fn preamble_limit_rejection_is_counted_when_handler_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("read test listener address");
    let client = TcpStream::connect(addr).await.expect("connect test client");
    let (mut stream, _) = listener.accept().await.expect("accept test client");
    let handler: PreambleFailure = Arc::new(|_, _| Box::pin(async { Ok(()) }));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(run_preamble_failure(
            Some(&handler),
            bincode::error::DecodeError::LimitExceeded,
            &mut stream,
            None,
        ));
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
            && matches!(value, DebugValue::Counter(count) if *count == 1)
    }));
    drop(client);
}
