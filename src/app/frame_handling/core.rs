//! Core frame handling context types.

use std::io;

use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use crate::{
    app::{codec_driver::FramePipeline, combined_codec::ConnectionCodec},
    codec::FrameCodec,
    serializer::Serializer,
};

/// Tracks deserialization failures and enforces a maximum error threshold.
pub(crate) struct DeserFailureTracker<'a> {
    /// Counter shared with the connection loop so failures accumulate per peer.
    count: &'a mut u32,
    /// Threshold at which malformed input terminates the connection.
    limit: u32,
}

impl<'a> DeserFailureTracker<'a> {
    /// Borrow the loop counter and configure the malformed-input threshold.
    pub(crate) fn new(count: &'a mut u32, limit: u32) -> Self { Self { count, limit } }

    /// Record one malformed frame and close the connection at the threshold.
    pub(super) fn record(
        &mut self,
        correlation_id: Option<u64>,
        context: &str,
        err: impl std::fmt::Debug,
    ) -> io::Result<()> {
        *self.count = (*self.count).saturating_add(1);
        warn!("{context}: correlation_id={correlation_id:?}, error={err:?}");
        crate::metrics::inc_deser_errors();
        if *self.count >= self.limit {
            warn!(
                "closing connection after {} deserialization failures: {context}",
                self.count
            );
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many deserialization failures",
            ));
        }
        Ok(())
    }
}

/// Bundles shared dependencies for response forwarding.
pub(crate) struct ResponseContext<'a, S, W, F>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    /// Serializer used to encode the handler response.
    pub(crate) serializer: &'a S,
    /// Framed transport receiving encoded response frames.
    pub(crate) framed: &'a mut Framed<W, ConnectionCodec<F>>,
    /// Outbound pipeline that applies fragmentation and metrics.
    pub(crate) pipeline: &'a mut FramePipeline,
    /// Codec that wraps serialized payloads into wire frames.
    pub(crate) codec: &'a F,
}
