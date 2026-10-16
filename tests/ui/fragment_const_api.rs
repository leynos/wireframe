//! Verifies public fragment and connection APIs remain callable in const contexts.

use wireframe::{
    connection::ConnectionChannels,
    fragment::{
        FragmentBatch,
        FragmentFrame,
        FragmentHeader,
        FragmentIndex,
        FragmentParts,
        MessageId,
        ReassembledMessage,
    },
    push::{PushHandle, PushQueues},
};

const MESSAGE_ID: MessageId = MessageId::new(7);
const HEADER: FragmentHeader = FragmentHeader::new(MESSAGE_ID, FragmentIndex::zero(), true);
const FRAME: FragmentFrame = FragmentFrame::new(HEADER, Vec::new());
const PARTS: FragmentParts = FragmentParts::new(3, Some(9), Vec::new());
const MESSAGE: ReassembledMessage = ReassembledMessage::new(MESSAGE_ID, Vec::new());
const HEADER_MESSAGE_ID: u64 = HEADER.message_id().get();
const HEADER_IS_LAST: bool = HEADER.is_last_fragment();

const fn inspect_frame(frame: &FragmentFrame) -> (u64, usize) {
    (frame.header().message_id().get(), frame.payload().len())
}

const fn inspect_parts(parts: &FragmentParts) -> (u32, Option<u64>) {
    (parts.id(), parts.correlation_id())
}

const fn inspect_message(message: &ReassembledMessage) -> (u64, usize) {
    (message.message_id().get(), message.payload().len())
}

const fn inspect_batch(batch: &FragmentBatch) -> (usize, usize, bool, bool) {
    (
        batch.fragments().len(),
        batch.len(),
        batch.is_empty(),
        batch.is_fragmented(),
    )
}

const fn bundle_channels<F>(queues: PushQueues<F>, handle: PushHandle<F>) -> ConnectionChannels<F> {
    ConnectionChannels::new(queues, handle)
}

fn main() {
    let _ = (
        HEADER_MESSAGE_ID,
        HEADER_IS_LAST,
        inspect_frame,
        inspect_parts,
        inspect_message,
        inspect_batch,
        bundle_channels::<u8>,
        FRAME,
        PARTS,
        MESSAGE,
    );
}
