//! Failure coverage for the second fragment of a fragmented send.
//!
//! A fragmented response that fails part-way leaves the peer with an
//! incomplete fragment series. These tests pin the contract that the failure
//! is reported, the remaining fragments are discarded rather than retried, and
//! the pipeline retains no output.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::codec::Framed;

use super::*;
use crate::{app::combined_codec::ConnectionCodec, connection::FrameSink};

/// Writer that accepts whole writes until a chosen attempt, then fails.
///
/// Each framed flush issues one write for the pending frame, so failing on the
/// second attempt models a transport that accepted the first fragment and then
/// lost the peer.
struct FailingWriter {
    /// Number of write attempts accepted before every later attempt fails.
    writes_before_failure: usize,
    /// Write attempts observed, including the failing one.
    attempts: usize,
}

impl FailingWriter {
    /// Build a writer that fails after `writes_before_failure` accepted writes.
    const fn new(writes_before_failure: usize) -> Self {
        Self {
            writes_before_failure,
            attempts: 0,
        }
    }

    /// Return the number of write attempts made so far.
    const fn attempts(&self) -> usize { self.attempts }
}

impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let attempt = this.attempts;
        this.attempts += 1;
        if attempt >= this.writes_before_failure {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "test writer loses the peer",
            )));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for FailingWriter {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Fragmentation settings whose payload cap splits a test envelope.
fn fragmenting_config() -> FragmentationConfig {
    let message_cap = NonZeroUsize::new(256).expect("message cap must be non-zero");
    FragmentationConfig::for_frame_budget(96, message_cap, Duration::from_secs(5))
        .expect("frame budget must exceed fragmentation overhead")
}

/// Payload long enough to produce more than one fragment under
/// [`fragmenting_config`].
fn multi_fragment_payload() -> Vec<u8> {
    let payload_cap = fragmenting_config().fragment_payload_cap.get();
    vec![7; payload_cap.saturating_add(16)]
}

/// Pipeline already holding one envelope split across several fragments.
fn fragmented_pipeline() -> FramePipeline<Bytes> {
    let mut pipeline = FramePipeline::new(Some(fragmenting_config()));
    pipeline
        .process(Envelope::new(1, None, multi_fragment_payload()))
        .expect("oversized envelope should fragment");
    pipeline
}

/// Framed transport whose second write fails.
fn failing_framed(codec: &TestCodec) -> Framed<FailingWriter, ConnectionCodec<TestCodec>> {
    Framed::new(
        FailingWriter::new(1),
        CombinedCodec::new(codec.decoder(), codec.encoder()),
    )
}

/// A failure on the second fragment is reported and abandons the rest.
#[tokio::test]
async fn flush_reports_a_failure_on_the_second_fragment() {
    let mut pipeline = fragmented_pipeline();
    let codec = TestCodec::default();
    let mut framed = failing_framed(&codec);
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        sink: &mut framed,
    };

    let error = flush_pipeline_output(&mut transport, &mut pipeline)
        .await
        .expect_err("a failure on the second fragment must be reported");

    // `send_envelope` wraps the transport error with `io::Error::other`, so
    // the original kind is not preserved; the message is.
    assert!(
        error.to_string().contains("test writer loses the peer"),
        "the transport failure must be preserved, got: {error}"
    );
    assert_eq!(
        framed.get_ref().attempts(),
        2,
        "the flush must stop at the failed write rather than retry"
    );
    assert!(
        pipeline.drain_output().is_empty(),
        "the remaining fragments must be discarded after the failed write"
    );
}

/// `send_frame` surfaces the same second-fragment failure to the actor.
#[tokio::test]
async fn send_frame_reports_a_failure_on_the_second_fragment() {
    // The sink fragments through the pipeline, so the pipeline must carry the
    // fragmentation config even though it starts empty.
    let mut pipeline: FramePipeline<Bytes> = FramePipeline::new(Some(fragmenting_config()));
    let codec = TestCodec::default();
    let mut framed = failing_framed(&codec);

    let error = {
        let mut sink = ActorFrameSink::new(&BincodeSerializer, &codec, &mut framed, &mut pipeline);
        sink.send_frame(Envelope::new(1, None, multi_fragment_payload()))
            .await
            .expect_err("a failure on the second fragment must be reported")
    };

    assert!(
        error.to_string().contains("test writer loses the peer"),
        "the sink must propagate the transport failure, got: {error}"
    );
    assert_eq!(
        framed.get_ref().attempts(),
        2,
        "the sink must stop at the failed write rather than retry"
    );
    assert!(
        pipeline.drain_output().is_empty(),
        "the sink must leave no fragments buffered for a later flush"
    );
}
