//! Test assembler that signals after a protocol frame header is parsed.

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use wireframe::message_assembler::{MessageAssembler, ParsedFrameHeader};

/// Assembler used when a scenario must advance virtual time after processing
/// a particular frame, rather than after merely writing it to the stream.
pub(crate) struct FrameProcessingAssembler {
    processed_tx: UnboundedSender<()>,
}

impl FrameProcessingAssembler {
    /// Create an assembler and the receiver used to await parsed frames.
    pub(crate) fn new() -> (Self, UnboundedReceiver<()>) {
        let (processed_tx, processed_rx) = mpsc::unbounded_channel();
        (Self { processed_tx }, processed_rx)
    }
}

impl MessageAssembler for FrameProcessingAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, std::io::Error> {
        let parsed = wireframe::test_helpers::parse_frame_header(payload)?;
        let _ = self.processed_tx.send(());
        Ok(parsed)
    }
}
