//! Test assembler that signals after a protocol frame header is parsed.

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use wireframe::message_assembler::{MessageAssembler, ParsedFrameHeader};

/// Scheduling rounds to poll for a signal before reporting it as delayed.
const SIGNAL_POLL_ATTEMPTS: usize = 64;

/// Outcome of polling for one frame-processing signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameSignal {
    /// The server parsed a frame header.
    Processed,
    /// The assembler was dropped without signalling.
    ChannelClosed,
    /// The signal was still pending when the bounded poll expired.
    Pending,
}

/// Poll for one frame-processing signal without advancing virtual time.
///
/// These fixtures pause the Tokio clock, and awaiting a timer under `pause()`
/// lets the runtime auto-advance to the next pending timer. A pacing delay
/// that regressed would then satisfy the wait and hide the regression. Yielding
/// keeps the clock frozen, so a delayed signal stays pending and the caller
/// reports it.
pub(crate) fn poll_frame_signal(
    runtime: &tokio::runtime::Runtime,
    processed_rx: &mut UnboundedReceiver<()>,
) -> FrameSignal {
    for _ in 0..SIGNAL_POLL_ATTEMPTS {
        match processed_rx.try_recv() {
            Ok(()) => return FrameSignal::Processed,
            Err(mpsc::error::TryRecvError::Disconnected) => return FrameSignal::ChannelClosed,
            Err(mpsc::error::TryRecvError::Empty) => {
                runtime.block_on(async { tokio::task::yield_now().await });
            }
        }
    }
    FrameSignal::Pending
}

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
