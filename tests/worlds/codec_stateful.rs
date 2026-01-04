//! Test world for stateful codec sequence counters.
//!
//! Ensures per-connection codec state is isolated so sequence numbers reset
//! between client connections.
#![cfg(not(loom))]

use std::{
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use cucumber::World;
use futures::{SinkExt, StreamExt};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::codec::{Decoder, Encoder, Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, WireframeApp},
    codec::FrameCodec,
    serializer::BincodeSerializer,
};

use super::TestResult;

#[derive(Debug)]
struct SeqFrame {
    sequence: u64,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct SeqFrameCodec {
    max_frame_length: usize,
    counter: AtomicU64,
}

impl SeqFrameCodec {
    fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            counter: AtomicU64::new(0),
        }
    }

    fn next_sequence(&self) -> u64 { self.counter.fetch_add(1, Ordering::SeqCst) + 1 }
}

impl Clone for SeqFrameCodec {
    fn clone(&self) -> Self {
        Self {
            max_frame_length: self.max_frame_length,
            counter: AtomicU64::new(0),
        }
    }
}

impl Default for SeqFrameCodec {
    fn default() -> Self { Self::new(1024) }
}

#[derive(Clone, Debug)]
struct SeqAdapter {
    inner: LengthDelimitedCodec,
    max_frame_length: usize,
}

impl SeqAdapter {
    fn new(max_frame_length: usize) -> Self {
        Self {
            inner: LengthDelimitedCodec::builder()
                .max_frame_length(max_frame_length)
                .new_codec(),
            max_frame_length,
        }
    }
}

impl Decoder for SeqAdapter {
    type Item = SeqFrame;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(mut bytes) = self.inner.decode(src)? else {
            return Ok(None);
        };
        if bytes.len() < 8 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame too short",
            ));
        }
        let sequence = bytes.get_u64();
        let payload = bytes.to_vec();
        Ok(Some(SeqFrame { sequence, payload }))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(mut bytes) = self.inner.decode_eof(src)? else {
            return Ok(None);
        };
        if bytes.len() < 8 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame too short",
            ));
        }
        let sequence = bytes.get_u64();
        let payload = bytes.to_vec();
        Ok(Some(SeqFrame { sequence, payload }))
    }
}

impl Encoder<SeqFrame> for SeqAdapter {
    type Error = std::io::Error;

    fn encode(&mut self, item: SeqFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let frame_len = item.payload.len().saturating_add(8);
        if frame_len > self.max_frame_length {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        let mut buf = BytesMut::with_capacity(frame_len);
        buf.put_u64(item.sequence);
        buf.extend_from_slice(&item.payload);
        self.inner.encode(buf.freeze(), dst)
    }
}

impl FrameCodec for SeqFrameCodec {
    type Frame = SeqFrame;
    type Decoder = SeqAdapter;
    type Encoder = SeqAdapter;

    fn decoder(&self) -> Self::Decoder { SeqAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { SeqAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        SeqFrame {
            sequence: self.next_sequence(),
            payload: payload.to_vec(),
        }
    }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

#[derive(Debug)]
struct StatefulServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
}

async fn serve_stateful_connections(
    listener: TcpListener,
    app: WireframeApp<BincodeSerializer, (), Envelope, SeqFrameCodec>,
) {
    for _ in 0..2 {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let _ = app.handle_connection_result(stream).await;
    }
}

#[derive(Debug, Default, World)]
/// Test world for stateful codec scenarios.
pub struct CodecStatefulWorld {
    server: Option<StatefulServer>,
    max_frame_length: usize,
    first_sequences: Vec<u64>,
    second_sequences: Vec<u64>,
}

impl CodecStatefulWorld {
    /// Start a server using the sequence-aware codec.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub async fn start_server(&mut self, max_frame_length: usize) -> TestResult {
        let app = WireframeApp::<BincodeSerializer, (), Envelope, SeqFrameCodec>::new()?
            .with_codec(SeqFrameCodec::new(max_frame_length))
            .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            serve_stateful_connections(listener, app).await;
        });

        self.server = Some(StatefulServer { addr, handle });
        self.max_frame_length = max_frame_length;
        Ok(())
    }

    /// Send requests on the first connection and store sequence numbers.
    ///
    /// # Errors
    /// Returns an error if the client cannot connect or exchange frames.
    pub async fn send_first_requests(&mut self, count: usize) -> TestResult {
        self.first_sequences = self.send_requests(count).await?;
        Ok(())
    }

    /// Send requests on the second connection and store sequence numbers.
    ///
    /// # Errors
    /// Returns an error if the client cannot connect or exchange frames.
    pub async fn send_second_requests(&mut self, count: usize) -> TestResult {
        self.second_sequences = self.send_requests(count).await?;
        Ok(())
    }

    /// Verify expected sequence numbers for the first connection.
    ///
    /// # Errors
    /// Returns an error if the observed sequence numbers do not match.
    pub async fn verify_first_sequences(&self, expected: &[u64]) -> TestResult {
        if self.first_sequences.as_slice() != expected {
            return Err(format!(
                "unexpected first connection sequences: {:?}",
                self.first_sequences
            )
            .into());
        }
        tokio::task::yield_now().await;
        Ok(())
    }

    /// Verify expected sequence numbers for the second connection.
    ///
    /// # Errors
    /// Returns an error if the observed sequence numbers do not match.
    pub async fn verify_second_sequences(&mut self, expected: &[u64]) -> TestResult {
        if self.second_sequences.as_slice() != expected {
            return Err(format!(
                "unexpected second connection sequences: {:?}",
                self.second_sequences
            )
            .into());
        }
        self.await_server().await?;
        Ok(())
    }

    async fn send_requests(&self, count: usize) -> TestResult<Vec<u64>> {
        let addr = self.server.as_ref().ok_or("server not started")?.addr;
        let stream = TcpStream::connect(addr).await?;
        let mut framed = Framed::new(stream, SeqAdapter::new(self.max_frame_length));
        let mut sequences = Vec::with_capacity(count);

        for _ in 0..count {
            let request = Envelope::new(1, None, b"ping".to_vec());
            let payload = BincodeSerializer.serialize(&request)?;
            framed
                .send(SeqFrame {
                    sequence: 0,
                    payload,
                })
                .await?;
            let frame = framed.next().await.ok_or("missing response frame")??;
            sequences.push(frame.sequence);
        }

        let mut stream = framed.into_inner();
        stream.shutdown().await?;
        Ok(sequences)
    }

    async fn await_server(&mut self) -> TestResult {
        if let Some(server) = self.server.take() {
            server
                .handle
                .await
                .map_err(|err| format!("server task failed: {err}"))?;
        }
        Ok(())
    }
}
