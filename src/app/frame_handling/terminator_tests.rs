//! Protocol-terminator preservation through the response path.
//!
//! A protocol terminator is a complete codec frame. This module proves the
//! inbound path sends the frame returned by
//! [`WireframeProtocol::stream_end_frame`] straight to the sink, so metadata a
//! custom codec keeps outside the payload survives. Decoding the terminator
//! into an [`Envelope`] and re-wrapping it would reset that metadata.
//!
//! The frame is not byte-for-byte unmodified: the connection-level
//! `before_send` hook still observes it and may rewrite it, exactly as it does
//! for application frames. The second scenario below pins that ordering.

use std::{
    convert::Infallible,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{Sink, StreamExt};
use rstest::rstest;
use tokio_util::{
    codec::{Decoder, Encoder},
    sync::CancellationToken,
};

use super::ResponseContext;
use crate::{
    app::{Envelope, codec_driver::FramePipeline},
    codec::FrameCodec,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse},
    push::{PushConfigError, PushHandle, PushQueues},
    response::{FrameStream, Response},
    serializer::BincodeSerializer,
};

/// Marker the test's `before_send` hook appends to the terminator payload.
const MARKER: u8 = 0x5a;
/// Tag the protocol assigns to its terminator, distinct from codec wrapping.
const TERMINATOR_TAG: u8 = 0xab;
/// Tag [`TaggedCodec::wrap_payload`] assigns to application frames.
const WRAPPED_TAG: u8 = 0x01;

/// Frame carrying protocol metadata in a tag byte outside the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaggedFrame {
    tag: u8,
    payload: Vec<u8>,
}

/// Codec whose wrapping tag differs from the protocol's terminator tag.
#[derive(Clone, Copy, Debug)]
struct TaggedCodec;

impl Decoder for TaggedCodec {
    type Item = TaggedFrame;
    type Error = io::Error;

    fn decode(&mut self, _src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "decoding is not used by this test",
        ))
    }
}

impl Encoder<TaggedFrame> for TaggedCodec {
    type Error = io::Error;

    fn encode(&mut self, item: TaggedFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.put_u8(item.tag);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for TaggedCodec {
    type Frame = TaggedFrame;
    type Decoder = Self;
    type Encoder = Self;

    fn decoder(&self) -> Self::Decoder { *self }

    fn encoder(&self) -> Self::Encoder { *self }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    /// Wraps a payload with the codec's own tag.
    ///
    /// The frame's identifier is derived from the payload length so that a
    /// re-wrapped terminator is distinguishable from the original frame.
    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        TaggedFrame {
            tag: WRAPPED_TAG,
            payload: payload.to_vec(),
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.tag)) }

    fn max_frame_length(&self) -> usize { 1024 }
}

/// Protocol that terminates streams with a frame carrying its own metadata.
struct TaggingProtocol;

impl WireframeProtocol for TaggingProtocol {
    type Frame = TaggedFrame;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        Some(TaggedFrame {
            tag: TERMINATOR_TAG,
            payload: vec![7],
        })
    }
}

/// Service that returns an already-finished stream response.
struct FinishedStreamService;

#[async_trait::async_trait]
impl Service for FinishedStreamService {
    type Error = Infallible;

    async fn call(&self, _req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let stream: FrameStream<Envelope, ()> = futures::stream::empty().boxed();
        Ok(ServiceResponse::with_deferred(
            Vec::new(),
            None,
            Arc::new(Mutex::new(Some(Response::Stream(stream)))),
        ))
    }
}

/// Sink that records every frame written through it.
#[derive(Default)]
struct RecordingSink {
    frames: Vec<TaggedFrame>,
}

impl Sink<TaggedFrame> for RecordingSink {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: TaggedFrame) -> Result<(), Self::Error> {
        self.get_mut().frames.push(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Build an untracked actor whose push queues stay open for the test.
///
/// # Errors
///
/// Returns [`PushConfigError`] if the queue capacities are rejected.
fn build_actor() -> Result<(ConnectionActor<Envelope, ()>, PushHandle<Envelope>), PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    let actor = ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle.clone()),
        None,
        CancellationToken::new(),
        ProtocolHooks::default(),
    );
    Ok((actor, handle))
}

/// Failure type shared by the scenarios, covering queue and transport errors.
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Forward a finished stream through the inbound response path.
///
/// Builds the actor, codec, and protocol shared by both terminator scenarios
/// and returns every frame that reached the sink.
///
/// # Errors
///
/// Returns the push-queue configuration failure, or the transport error that
/// [`super::forward_response`] reports.
async fn forward_finished_stream(
    hooks: ProtocolHooks<TaggedFrame, ()>,
) -> Result<Vec<TaggedFrame>, Box<dyn std::error::Error + Send + Sync>> {
    let (mut actor, _handle) = build_actor()?;
    let codec = TaggedCodec;
    let serializer = BincodeSerializer;
    let mut sink = RecordingSink::default();
    let mut pipeline: FramePipeline<TaggedFrame> = FramePipeline::with_hooks(None, hooks);
    let service = HandlerService::<Envelope>::from_service(7, FinishedStreamService);
    let protocol: Arc<dyn WireframeProtocol<Frame = TaggedFrame, ProtocolError = ()>> =
        Arc::new(TaggingProtocol);

    super::forward_response(
        Envelope::new(7, Some(3), Vec::new()),
        &service,
        ResponseContext {
            serializer: &serializer,
            sink: &mut sink,
            pipeline: &mut pipeline,
            codec: &codec,
            actor: &mut actor,
            protocol: Some(&protocol),
        },
    )
    .await?;
    Ok(sink.frames)
}

/// Hooks that append [`MARKER`] to every frame the pipeline sends.
fn marking_hooks() -> ProtocolHooks<TaggedFrame, ()> {
    ProtocolHooks {
        before_send: Some(Box::new(
            |frame: &mut TaggedFrame, _ctx: &mut ConnectionContext| {
                frame.payload.push(MARKER);
            },
        )),
        ..ProtocolHooks::default()
    }
}

/// The protocol terminator reaches the sink with its original metadata.
#[rstest]
#[tokio::test]
async fn stream_terminator_is_written_without_decoding() -> TestResult {
    let frames = forward_finished_stream(ProtocolHooks::default()).await?;

    assert_eq!(
        frames,
        vec![TaggedFrame {
            tag: TERMINATOR_TAG,
            payload: vec![7],
        }],
        "the terminator must keep the protocol's tag rather than being decoded and re-wrapped"
    );
    Ok(())
}

/// The connection's `before_send` hook still observes the native terminator.
#[rstest]
#[tokio::test]
async fn stream_terminator_passes_through_before_send() -> TestResult {
    let frames = forward_finished_stream(marking_hooks()).await?;

    assert_eq!(
        frames,
        vec![TaggedFrame {
            tag: TERMINATOR_TAG,
            payload: vec![7, MARKER],
        }],
        "before_send must run on the frame the protocol returned"
    );
    Ok(())
}
