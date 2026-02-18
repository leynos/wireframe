//! Connection handling and response utilities for `WireframeApp`.

use std::{collections::HashMap, sync::Arc};

use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use log::{debug, warn};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, AsyncWriteExt},
    time::{Duration, timeout},
};
use tokio_util::codec::{Encoder, Framed, LengthDelimitedCodec};

use super::{
    builder::WireframeApp,
    codec_driver::FramePipeline,
    combined_codec::{CombinedCodec, ConnectionCodec},
    envelope::{Envelope, Packet},
    error::SendError,
    frame_handling,
};
use crate::{
    codec::{FrameCodec, LengthDelimitedFrameCodec, MAX_FRAME_LENGTH, clamp_frame_length},
    frame::FrameMetadata,
    message::Message,
    message_assembler::MessageAssemblyState,
    middleware::HandlerService,
    serializer::Serializer,
};

fn purge_expired(
    pipeline: &mut FramePipeline,
    message_assembly: &mut Option<MessageAssemblyState>,
) {
    pipeline.purge_expired();
    frame_handling::purge_expired_assemblies(message_assembly);
}
/// Maximum consecutive deserialization failures before closing a connection.
const MAX_DESER_FAILURES: u32 = 10;

/// Per-frame processing state bundled for `handle_frame`.
struct FrameHandlingContext<'a, E, W, F>
where
    E: Packet,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    framed: &'a mut Framed<W, ConnectionCodec<F>>,
    deser_failures: &'a mut u32,
    routes: &'a HashMap<u32, HandlerService<E>>,
    pipeline: &'a mut FramePipeline,
    message_assembly: &'a mut Option<MessageAssemblyState>,
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Serialize `msg` and write it to `stream` using the configured codec.
    ///
    /// # Errors
    ///
    /// Returns a [`SendError`] if serialization or writing fails.
    pub async fn send_response<W, M>(
        &self,
        stream: &mut W,
        msg: &M,
    ) -> std::result::Result<(), SendError>
    where
        W: AsyncWrite + Unpin,
        M: Message,
    {
        let bytes = self
            .serializer
            .serialize(msg)
            .map_err(SendError::Serialize)?;
        let payload_len = bytes.len();
        let frame = self.codec.wrap_payload(Bytes::from(bytes));
        let mut encoder = self.codec.encoder();
        let mut encoded_buf = BytesMut::with_capacity(payload_len);
        encoder
            .encode(frame, &mut encoded_buf)
            .map_err(SendError::Io)?;
        stream
            .write_all(&encoded_buf)
            .await
            .map_err(SendError::Io)?;
        stream.flush().await.map_err(SendError::Io)
    }

    /// Serialize `msg` and send it through an existing framed stream.
    ///
    /// # Errors
    ///
    /// Returns a [`SendError`] if serialization or sending fails.
    pub async fn send_response_framed_with_codec<W, M, Cc>(
        &self,
        framed: &mut Framed<W, Cc>,
        msg: &M,
    ) -> std::result::Result<(), SendError>
    where
        W: AsyncRead + AsyncWrite + Unpin,
        M: Message,
        Cc: Encoder<F::Frame, Error = io::Error>,
    {
        let bytes = self
            .serializer
            .serialize(msg)
            .map_err(SendError::Serialize)?;
        let frame = self.codec.wrap_payload(Bytes::from(bytes));
        framed.send(frame).await.map_err(SendError::Io)
    }
}

impl<S, C, E> WireframeApp<S, C, E, LengthDelimitedFrameCodec>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    /// Construct a length-delimited codec capped by the application's buffer
    /// capacity.
    #[must_use]
    pub fn length_codec(&self) -> LengthDelimitedCodec {
        LengthDelimitedCodec::builder()
            .max_frame_length(self.codec.max_frame_length())
            .new_codec()
    }

    /// Serialize `msg` and send it through an existing framed stream.
    ///
    /// # Errors
    ///
    /// Returns a [`SendError`] if serialization or sending fails.
    pub async fn send_response_framed<W, M>(
        &self,
        framed: &mut Framed<W, LengthDelimitedCodec>,
        msg: &M,
    ) -> std::result::Result<(), SendError>
    where
        W: AsyncRead + AsyncWrite + Unpin,
        M: Message,
    {
        let bytes = self
            .serializer
            .serialize(msg)
            .map_err(SendError::Serialize)?;
        framed.send(bytes.into()).await.map_err(SendError::Io)
    }
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Try parsing the frame using [`FrameMetadata::parse`], falling back to
    /// full deserialization on failure.
    fn parse_envelope(
        &self,
        payload: &[u8],
    ) -> std::result::Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>> {
        self.serializer
            .parse(payload)
            .map_err(Box::<dyn std::error::Error + Send + Sync>::from)
            .or_else(|_| self.serializer.deserialize::<Envelope>(payload))
    }

    /// Handle an accepted connection end-to-end, returning any processing error.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    pub async fn handle_connection_result<W>(&self, stream: W) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let state = if let Some(setup) = &self.on_connect {
            Some((setup)().await)
        } else {
            None
        };

        let routes = self
            .routes
            .get_or_init(|| async { Arc::new(self.build_chains().await) })
            .await
            .clone();

        if let Err(e) = self.process_stream(stream, &routes).await {
            warn!(
                "connection terminated with error: correlation_id={:?}, error={e:?}",
                None::<u64>
            );
            return Err(e);
        }

        if let (Some(teardown), Some(state)) = (&self.on_disconnect, state) {
            teardown(state).await;
        }

        Ok(())
    }

    /// Handle an accepted connection end-to-end, logging errors and swallowing the result.
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(e) = self.handle_connection_result(stream).await {
            warn!(
                "connection handling completed with error: correlation_id={:?}, error={e:?}",
                None::<u64>
            );
        }
    }

    async fn build_chains(&self) -> HashMap<u32, HandlerService<E>> {
        let mut routes = HashMap::new();
        for (&id, handler) in &self.handlers {
            let mut service = HandlerService::new(id, handler.clone());
            for mw in self.middleware.iter().rev() {
                service = mw.transform(service).await;
            }
            routes.insert(id, service);
        }
        routes
    }

    async fn process_stream<W>(
        &self,
        stream: W,
        routes: &Arc<HashMap<u32, HandlerService<E>>>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        let codec = self.codec.clone();
        let combined = CombinedCodec::new(codec.decoder(), codec.encoder());
        let mut framed = Framed::new(stream, combined);
        let requested_frame_length = codec.max_frame_length();
        let max_frame_length = clamp_frame_length(requested_frame_length);
        if requested_frame_length > MAX_FRAME_LENGTH {
            warn!(
                "codec max frame length exceeds guardrail; clamping to {MAX_FRAME_LENGTH} bytes \
                 (requested={requested_frame_length})"
            );
        }
        framed.read_buffer_mut().reserve(max_frame_length);
        let mut deser_failures = 0u32;
        let mut message_assembly = self.message_assembler.as_ref().map(|_| {
            frame_handling::new_message_assembly_state(self.fragmentation, requested_frame_length)
        });
        let mut pipeline = FramePipeline::new(self.fragmentation);
        let timeout_dur = Duration::from_millis(self.read_timeout_ms);

        loop {
            match timeout(timeout_dur, framed.next()).await {
                Ok(Some(Ok(frame))) => {
                    self.handle_frame(
                        &frame,
                        FrameHandlingContext {
                            framed: &mut framed,
                            deser_failures: &mut deser_failures,
                            routes,
                            message_assembly: &mut message_assembly,
                            pipeline: &mut pipeline,
                        },
                        &codec,
                    )
                    .await?;
                }
                Ok(Some(Err(e))) => return Err(e),
                Ok(None) => break,
                Err(_) => {
                    debug!("read timeout elapsed; continuing to wait for next frame");
                    purge_expired(&mut pipeline, &mut message_assembly);
                }
            }
        }

        Ok(())
    }

    async fn handle_frame<W>(
        &self,
        frame: &F::Frame,
        ctx: FrameHandlingContext<'_, E, W, F>,
        codec: &F,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        let FrameHandlingContext {
            framed,
            deser_failures,
            routes,
            message_assembly,
            pipeline,
        } = ctx;

        crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
        let Some(env) = frame_handling::decode_envelope::<F>(
            self.parse_envelope(F::frame_payload(frame)),
            frame,
            deser_failures,
            MAX_DESER_FAILURES,
        )?
        else {
            return Ok(());
        };
        let Some(env) = frame_handling::reassemble_if_needed(
            pipeline,
            deser_failures,
            env,
            MAX_DESER_FAILURES,
        )?
        else {
            return Ok(());
        };
        let Some(env) = frame_handling::assemble_if_needed(
            frame_handling::AssemblyRuntime::new(self.message_assembler.as_ref(), message_assembly),
            deser_failures,
            env,
            MAX_DESER_FAILURES,
        )?
        else {
            return Ok(());
        };

        // Reset failure counter only after the entire inbound pipeline
        // (decode, reassemble, assemble) succeeds, so that assembly-stage
        // failures accumulate towards the threshold.
        *deser_failures = 0;

        if let Some(service) = routes.get(&env.id) {
            frame_handling::forward_response(
                env,
                service,
                frame_handling::ResponseContext::<S, W, F> {
                    serializer: &self.serializer,
                    framed,
                    pipeline,
                    codec,
                },
            )
            .await?;
        } else {
            warn!(
                "no handler for message id: id={}, correlation_id={:?}",
                env.id, env.correlation_id
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
