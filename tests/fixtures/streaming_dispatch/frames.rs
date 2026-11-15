//! Frame construction and client I/O for the streaming dispatch world.

use std::{io, time::Duration};

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use wireframe::{
    app::Envelope,
    message_assembler::{FrameSequence, MessageKey},
    serializer::{BincodeSerializer, Serializer},
    test_helpers,
};

use super::{
    BUFFERED_ROUTE,
    CORRELATION_ID,
    READ_TIMEOUT_MS,
    STREAM_ROUTE,
    StreamingDispatchWorld,
    TestResult,
};

impl StreamingDispatchWorld {
    /// Send a streaming first frame, optionally declaring the body length.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_streaming_first(
        &mut self,
        key: impl Into<MessageKey>,
        body: &str,
        total: Option<u32>,
    ) -> TestResult {
        let payload = test_helpers::first_frame_payload(key.into(), body.as_bytes(), false, total)?;
        self.send_message(STREAM_ROUTE, payload)
    }

    /// Send a streaming continuation frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_streaming_continuation(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        let payload = test_helpers::continuation_frame_payload(
            key.into(),
            sequence.into(),
            body.as_bytes(),
            is_last,
        )?;
        self.send_message(STREAM_ROUTE, payload)
    }

    /// Send a buffered first frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_buffered_first(
        &mut self,
        key: impl Into<MessageKey>,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        let total = is_last.then(|| u32::try_from(body.len()).unwrap_or(u32::MAX));
        let payload =
            test_helpers::first_frame_payload(key.into(), body.as_bytes(), is_last, total)?;
        self.send_message(BUFFERED_ROUTE, payload)
    }

    /// Send a buffered continuation frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_buffered_continuation(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        let payload = test_helpers::continuation_frame_payload(
            key.into(),
            sequence.into(),
            body.as_bytes(),
            is_last,
        )?;
        self.send_message(BUFFERED_ROUTE, payload)
    }

    /// Send the first part of a complete buffered message, retaining the rest.
    ///
    /// The split lands inside the transport length prefix so the server sees a
    /// paused frame header. The runtime is then given time to reach a read
    /// timeout with the partial frame buffered.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_split_buffered_first(
        &mut self,
        key: impl Into<MessageKey>,
        body: &str,
    ) -> TestResult {
        let total = u32::try_from(body.len()).unwrap_or(u32::MAX);
        let payload =
            test_helpers::first_frame_payload(key.into(), body.as_bytes(), true, Some(total))?;
        let framed = transport_frame(&self.encode_message(BUFFERED_ROUTE, &payload)?)?;
        let split = framed.len().min(2);
        let (head, tail) = framed.split_at(split);
        self.write_raw(head)?;
        self.pending = tail.to_vec();
        self.wait_millis(3 * READ_TIMEOUT_MS / 2)
    }

    /// Write the retained remainder of a split frame.
    ///
    /// # Errors
    ///
    /// Returns an error if nothing was retained or the write fails.
    pub fn send_pending_remainder(&mut self) -> TestResult {
        let remainder = std::mem::take(&mut self.pending);
        if remainder.is_empty() {
            return Err("no partial frame is pending".into());
        }
        self.write_raw(&remainder)
    }

    /// Release a handler waiting in [`HandlerMode::Hold`].
    pub fn release_handler(&mut self) {
        if let Some(gate) = self.gate.as_ref() {
            gate.add_permits(1);
        }
    }

    /// Let wall-clock time pass while the connection runtime keeps running.
    ///
    /// Assembly expiry uses `std::time::Instant`, so the wait must be real
    /// time rather than an advanced virtual clock.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime is unavailable.
    pub fn wait_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async move { tokio::time::sleep(Duration::from_millis(millis)).await })
    }

    fn send_message(&mut self, route: u32, payload: Vec<u8>) -> TestResult {
        let framed = transport_frame(&self.encode_message(route, &payload)?)?;
        self.write_raw(&framed)
    }

    fn encode_message(&self, route: u32, payload: &[u8]) -> TestResult<Vec<u8>> {
        let envelope = Envelope::new(route, CORRELATION_ID, payload.to_vec());
        Ok(BincodeSerializer.serialize(&envelope)?)
    }

    /// Write raw transport bytes and let the connection runtime make progress.
    fn write_raw(&mut self, bytes: &[u8]) -> TestResult {
        let mut client = self.client.take().ok_or("client not initialized")?;
        let result = self.block_on(async {
            client.write_all(bytes).await?;
            client.flush().await
        });
        self.client = Some(client);
        result??;
        self.block_on(async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        })?;
        Ok(())
    }
}

/// Prefix serialized envelope bytes with their transport length.
fn transport_frame(envelope: &[u8]) -> TestResult<Vec<u8>> {
    let len = u32::try_from(envelope.len()).map_err(|_| "frame too large")?;
    let mut framed = Vec::with_capacity(envelope.len() + 4);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(envelope);
    Ok(framed)
}

/// Read one length-delimited reply frame from the client side.
pub(super) async fn read_frame(client: &mut DuplexStream) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    client.read_exact(&mut len).await?;
    let mut payload = vec![0u8; u32::from_be_bytes(len) as usize];
    client.read_exact(&mut payload).await?;
    Ok(payload)
}
