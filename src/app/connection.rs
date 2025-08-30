//! Connection handling and response utilities for `WireframeApp`.

use std::{collections::HashMap, sync::Arc};

use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, AsyncWriteExt},
    time::{Duration, timeout},
};
use tokio_util::codec::{Encoder, Framed, LengthDelimitedCodec};

use super::{
    builder::WireframeApp,
    envelope::{Envelope, Packet, PacketParts},
    error::SendError,
};
use crate::{
    frame::FrameMetadata,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest},
    serializer::Serializer,
};

/// Maximum consecutive deserialization failures before closing a connection.
const MAX_DESER_FAILURES: u32 = 10;

#[derive(Debug)]
enum EnvelopeDecodeError<E> {
    Parse(E),
    Deserialize(Box<dyn std::error::Error + Send + Sync>),
}

impl<S, C, E> WireframeApp<S, C, E>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    fn new_length_codec(&self) -> LengthDelimitedCodec {
        LengthDelimitedCodec::builder()
            .max_frame_length(self.buffer_capacity)
            .new_codec()
    }

    /// Serialize `msg` and write it to `stream` using a length-delimited codec.
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
        let mut codec = self.new_length_codec();
        let mut framed = BytesMut::new();
        codec
            .encode(bytes.into(), &mut framed)
            .map_err(|e| SendError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
        stream.write_all(&framed).await.map_err(SendError::Io)?;
        stream.flush().await.map_err(SendError::Io)
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

impl<S, C, E> WireframeApp<S, C, E>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    /// Try parsing the frame using [`FrameMetadata::parse`], falling back to
    /// full deserialization on failure.
    fn parse_envelope(
        &self,
        frame: &[u8],
    ) -> std::result::Result<(Envelope, usize), EnvelopeDecodeError<S::Error>> {
        self.serializer
            .parse(frame)
            .map_err(EnvelopeDecodeError::Parse)
            .or_else(|_| {
                self.serializer
                    .deserialize::<Envelope>(frame)
                    .map_err(EnvelopeDecodeError::Deserialize)
            })
    }

    /// Handle an accepted connection end-to-end.
    ///
    /// Runs optional connection setup to produce per-connection state,
    /// initializes (and caches) route chains, processes the framed stream
    /// with per-frame timeouts, and finally runs optional teardown.
    pub async fn handle_connection<W>(&self, stream: W)
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
            tracing::warn!(correlation_id = ?None::<u64>, error = ?e, "connection terminated with error");
        }

        if let (Some(teardown), Some(state)) = (&self.on_disconnect, state) {
            teardown(state).await;
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
        let codec = self.new_length_codec();
        let mut framed = Framed::new(stream, codec);
        framed.read_buffer_mut().reserve(self.buffer_capacity);
        let mut deser_failures = 0u32;
        let timeout_dur = Duration::from_millis(self.read_timeout_ms);

        loop {
            match timeout(timeout_dur, framed.next()).await {
                Ok(Some(Ok(buf))) => {
                    self.handle_frame(&mut framed, buf.as_ref(), &mut deser_failures, routes)
                        .await?;
                }
                Ok(Some(Err(e))) => return Err(e),
                Ok(None) => break,
                Err(_) => {
                    tracing::debug!("read timeout elapsed; continuing to wait for next frame");
                }
            }
        }

        Ok(())
    }

    async fn handle_frame<W>(
        &self,
        framed: &mut Framed<W, LengthDelimitedCodec>,
        frame: &[u8],
        deser_failures: &mut u32,
        routes: &HashMap<u32, HandlerService<E>>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
        let (env, _) = match self.parse_envelope(frame) {
            Ok(result) => {
                *deser_failures = 0;
                result
            }
            Err(EnvelopeDecodeError::Parse(e)) => {
                *deser_failures += 1;
                tracing::warn!(correlation_id = ?None::<u64>, error = ?e, "failed to parse message");
                crate::metrics::inc_deser_errors();
                if *deser_failures >= MAX_DESER_FAILURES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many deserialization failures",
                    ));
                }
                return Ok(());
            }
            Err(EnvelopeDecodeError::Deserialize(e)) => {
                *deser_failures += 1;
                tracing::warn!(correlation_id = ?None::<u64>, error = ?e, "failed to deserialize message");
                crate::metrics::inc_deser_errors();
                if *deser_failures >= MAX_DESER_FAILURES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many deserialization failures",
                    ));
                }
                return Ok(());
            }
        };

        if let Some(service) = routes.get(&env.id) {
            let request = ServiceRequest::new(env.payload, env.correlation_id);
            match service.call(request).await {
                Ok(resp) => {
                    let parts = PacketParts::new(env.id, resp.correlation_id(), resp.into_inner())
                        .inherit_correlation(env.correlation_id);
                    let correlation_id = parts.correlation_id();
                    let response = Envelope::from_parts(parts);
                    match self.serializer.serialize(&response) {
                        Ok(bytes) => {
                            if let Err(e) = framed.send(bytes.into()).await {
                                tracing::warn!(
                                    id = env.id,
                                    correlation_id = ?correlation_id,
                                    error = ?e,
                                    "failed to send response",
                                );
                                crate::metrics::inc_handler_errors();
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                id = env.id,
                                correlation_id = ?correlation_id,
                                error = ?e,
                                "failed to serialize response",
                            );
                            crate::metrics::inc_handler_errors();
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(id = env.id, correlation_id = ?env.correlation_id, error = ?e, "handler error");
                    crate::metrics::inc_handler_errors();
                }
            }
        } else {
            tracing::warn!(
                id = env.id,
                correlation_id = ?env.correlation_id,
                "no handler for message id"
            );
        }

        Ok(())
    }
}
