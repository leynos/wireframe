//! `on_eof` receives exactly the bytes a custom decoder left behind.
//!
//! The documented contract says `partial_data` is whatever the framed reader
//! still held for the incomplete frame. The built-in codec peeks, so for it
//! that is every byte that arrived. A custom decoder may instead take the
//! length prefix as soon as it is complete, and this scenario pins what the
//! hook then observes: only the tail the decoder did not consume.

use std::{io, sync::Arc};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_util::codec::Decoder;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::{
        CodecError,
        EofError,
        FrameCodec,
        LENGTH_HEADER_SIZE,
        LengthDelimitedEncoder,
        LengthDelimitedFrameCodec,
    },
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

use super::harness::{await_connection_end, ended_notifier, ensure, start_server};
use crate::hook_recorder::{Event, HookLog, RecordingProtocol};

/// Declared payload length of the frame the peer abandons.
const DECLARED_LENGTH: usize = 8;
/// Payload bytes the peer sends before it disconnects.
const SENT_PAYLOAD: [u8; 3] = [1, 2, 3];
/// Maximum frame length the test codec accepts.
const MAX_FRAME_LENGTH: usize = 1024;

/// Codec whose decoder consumes the length prefix before the peer disconnects.
#[derive(Clone, Default)]
struct ConsumingCodec;

impl FrameCodec for ConsumingCodec {
    type Frame = Bytes;
    type Decoder = ConsumingDecoder;
    type Encoder = LengthDelimitedEncoder;

    fn decoder(&self) -> Self::Decoder { ConsumingDecoder::default() }

    fn encoder(&self) -> Self::Encoder { LengthDelimitedFrameCodec::default().encoder() }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_ref() }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Bytes { payload }

    fn max_frame_length(&self) -> usize { MAX_FRAME_LENGTH }
}

/// Decoder that takes the header as soon as it is complete.
///
/// It is the opposite of the built-in decoder, which peeks so the whole
/// incomplete frame stays in the read buffer for the hook to report.
#[derive(Default)]
struct ConsumingDecoder {
    /// Payload length the consumed header declared.
    expected: Option<usize>,
}

impl Decoder for ConsumingDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.expected.is_none() {
            if src.len() < LENGTH_HEADER_SIZE {
                return Ok(None);
            }
            // `get_u32` reads the prefix and consumes it, so no later report
            // can recover those bytes.
            let declared = src.get_u32();
            self.expected = Some(usize::try_from(declared).expect("a u32 length fits a usize"));
            return Ok(None);
        }
        let expected = self.expected.expect("a header was consumed above");
        if src.len() < expected {
            return Ok(None);
        }
        let payload = src.split_to(expected).freeze();
        self.expected = None;
        Ok(Some(payload))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.expected {
            Some(expected) => Err(CodecError::Eof(EofError::MidFrame {
                bytes_received: src.len(),
                expected,
            })
            .into()),
            None if src.is_empty() => Ok(None),
            None => Err(CodecError::Eof(EofError::MidHeader {
                bytes_received: src.len(),
                header_size: LENGTH_HEADER_SIZE,
            })
            .into()),
        }
    }
}

/// The hook sees only the tail a consuming decoder left unconsumed.
#[tokio::test]
async fn consuming_decoder_reports_only_the_unconsumed_tail() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let app = WireframeApp::<BincodeSerializer, (), Envelope, ConsumingCodec>::default()
        .with_protocol(RecordingProtocol::new(Arc::clone(&log), None))
        .on_connection_setup(|| async {})?
        .on_connection_teardown(ended_notifier(ended_tx))?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    let mut header = BytesMut::new();
    header.put_u32(u32::try_from(DECLARED_LENGTH).expect("a small length fits a u32"));
    client.write_all(&header).await?;
    client.write_all(&SENT_PAYLOAD).await?;
    client.shutdown().await?;
    await_connection_end(ended_rx).await?;

    let events = log.events();
    ensure(
        events
            == vec![
                Event::Setup,
                Event::Eof {
                    error: EofError::MidFrame {
                        bytes_received: SENT_PAYLOAD.len(),
                        expected: DECLARED_LENGTH,
                    },
                    partial: SENT_PAYLOAD.to_vec(),
                },
            ],
        format!("a consuming decoder leaves only its tail for on_eof: {events:?}"),
    )?;

    drop(client);
    server.stop().await
}
