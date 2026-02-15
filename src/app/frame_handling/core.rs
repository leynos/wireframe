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
pub(super) struct DeserFailureTracker<'a> {
    count: &'a mut u32,
    limit: u32,
}

impl<'a> DeserFailureTracker<'a> {
    pub(super) fn new(count: &'a mut u32, limit: u32) -> Self { Self { count, limit } }

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
    pub(crate) serializer: &'a S,
    pub(crate) framed: &'a mut Framed<W, ConnectionCodec<F>>,
    pub(crate) pipeline: &'a mut FramePipeline,
    pub(crate) codec: &'a F,
}
