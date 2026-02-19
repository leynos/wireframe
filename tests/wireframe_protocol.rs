//! Integration tests for the `WireframeProtocol` trait.
//!
//! These tests ensure that protocol implementations integrate correctly with
//! [`WireframeApp`] and [`ConnectionActor`]. They verify that hooks are invoked
//! with the expected connection context and that frame mutations occur as
//! intended.
#![cfg(not(loom))]

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::{Bytes, BytesMut};
use futures::stream;
use rstest::{fixture, rstest};
use tokio_util::{
    codec::{Decoder, Encoder, LengthDelimitedCodec},
    sync::CancellationToken,
};
use wireframe::{
    app::Envelope,
    codec::FrameCodec,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, WireframeProtocol},
    push::{PushConfigError, PushQueues},
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope, VecFrameCodec>;
type QueueResult =
    Result<(PushQueues<Vec<u8>>, wireframe::push::PushHandle<Vec<u8>>), PushConfigError>;

#[fixture]
fn queues() -> QueueResult {
    PushQueues::<Vec<u8>>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()
}

#[derive(Clone, Debug)]
struct VecFrameCodec {
    max_frame_length: usize,
}

impl VecFrameCodec {
    fn new_codec(&self) -> LengthDelimitedCodec {
        LengthDelimitedCodec::builder()
            .max_frame_length(self.max_frame_length)
            .new_codec()
    }
}

impl Default for VecFrameCodec {
    fn default() -> Self {
        Self {
            max_frame_length: 1024,
        }
    }
}

#[derive(Clone, Debug)]
struct VecDecoder {
    inner: LengthDelimitedCodec,
}

#[derive(Clone, Debug)]
struct VecEncoder {
    inner: LengthDelimitedCodec,
    max_frame_length: usize,
}

impl Decoder for VecDecoder {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.inner
            .decode(src)
            .map(|opt| opt.map(|bytes| bytes.to_vec()))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.inner
            .decode_eof(src)
            .map(|opt| opt.map(|bytes| bytes.to_vec()))
    }
}

impl Encoder<Vec<u8>> for VecEncoder {
    type Error = io::Error;

    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.len() > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        self.inner.encode(Bytes::from(item), dst)
    }
}

impl FrameCodec for VecFrameCodec {
    type Frame = Vec<u8>;
    type Decoder = VecDecoder;
    type Encoder = VecEncoder;

    fn decoder(&self) -> Self::Decoder {
        VecDecoder {
            inner: self.new_codec(),
        }
    }

    fn encoder(&self) -> Self::Encoder {
        VecEncoder {
            inner: self.new_codec(),
            max_frame_length: self.max_frame_length,
        }
    }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { payload.to_vec() }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

struct TestProtocol {
    counter: Arc<AtomicUsize>,
}

impl WireframeProtocol for TestProtocol {
    type Frame = Vec<u8>;
    type ProtocolError = ();

    fn on_connection_setup(
        &self,
        _handle: wireframe::push::PushHandle<Self::Frame>,
        _ctx: &mut ConnectionContext,
    ) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }

    fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) { frame.push(1); }

    fn on_command_end(&self, _ctx: &mut ConnectionContext) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }
}

#[rstest]
#[tokio::test]
async fn builder_produces_protocol_hooks(queues: QueueResult) -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let protocol = TestProtocol {
        counter: counter.clone(),
    };
    let app = TestApp::new()?.with_protocol(protocol);
    let mut hooks = app.protocol_hooks();
    let (_queues, handle) = queues?;
    hooks.on_connection_setup(handle, &mut ConnectionContext);

    let mut frame = vec![1u8];
    hooks.before_send(&mut frame, &mut ConnectionContext);
    hooks.on_command_end(&mut ConnectionContext);

    assert_eq!(frame, vec![1, 1], "before_send did not mutate frame");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "expected two protocol callbacks"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn connection_actor_uses_protocol_from_builder(queues: QueueResult) -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let protocol = TestProtocol {
        counter: counter.clone(),
    };
    let app = TestApp::new()?.with_protocol(protocol);

    let hooks = app.protocol_hooks();
    let (queues, handle) = queues?;
    handle
        .push_high_priority(vec![1])
        .await
        .map_err(|e| std::io::Error::other(format!("push failed: {e}")))?;
    let stream = stream::iter(vec![Ok(vec![2u8])]);
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        Some(Box::pin(stream)),
        CancellationToken::new(),
        hooks,
    );
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    assert_eq!(
        out,
        vec![vec![1, 1], vec![2, 1]],
        "frames not mutated as expected"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "expected two protocol callbacks"
    );
    Ok(())
}
