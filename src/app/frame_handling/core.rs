//! Core frame handling context types.

use std::{io, sync::Arc};

use futures::Sink;
use log::warn;

use crate::{
    app::{Envelope, codec_driver::FramePipeline},
    codec::FrameCodec,
    connection::ConnectionActor,
    hooks::WireframeProtocol,
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
    pub(crate) const fn new(count: &'a mut u32, limit: u32) -> Self { Self { count, limit } }

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
pub(crate) struct ResponseContext<'a, S, K, F>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin,
    F: FrameCodec,
{
    /// Serializer used to encode the handler response.
    pub(crate) serializer: &'a S,
    /// Sink receiving encoded response frames.
    pub(crate) sink: &'a mut K,
    /// Outbound pipeline that applies fragmentation, `before_send`, and
    /// metrics.
    pub(crate) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Codec that wraps serialized payloads into wire frames.
    pub(crate) codec: &'a F,
    /// Connection actor that schedules and delivers the response.
    pub(crate) actor: &'a mut ConnectionActor<Envelope, ()>,
    /// Protocol whose terminator and completion hooks follow the response.
    pub(crate) protocol:
        Option<&'a Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
}
