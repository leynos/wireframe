//! Connection handling and response utilities for `WireframeApp`.

use std::{collections::HashMap, sync::Arc};

use bincode::error::DecodeError;
use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use log::{debug, warn};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, AsyncWriteExt},
    time::{Duration, timeout},
};
use tokio_util::codec::{Encoder, Framed, LengthDelimitedCodec};

use super::{
    builder::{default_fragmentation, WireframeApp},
    envelope::{Envelope, Packet, PacketParts},
    error::SendError,
    fragment_utils::fragment_packet,
};
use crate::{
    fragment::{
        FragmentationConfig,
        FragmentationError,
        Fragmenter,
        MessageId,
        Reassembler,
        ReassemblyError,
        decode_fragment_payload,
        encode_fragment_payload,
    },
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

/// Bundles outbound fragmentation and inbound re-assembly state for a connection.
struct FragmentationState {
    fragmenter: Fragmenter,
    reassembler: Reassembler,
}

enum FragmentProcessError {
    Decode(DecodeError),
    Reassembly(ReassemblyError),
}

impl FragmentationState {
    fn new(config: FragmentationConfig) -> Self {
        Self {
            fragmenter: Fragmenter::new(config.fragment_payload_cap),
            reassembler: Reassembler::new(config.max_message_size, config.reassembly_timeout),
        }
    }

    fn fragment<E: Packet>(&self, packet: E) -> Result<Vec<E>, FragmentationError> {
        fragment_packet(&self.fragmenter, packet)
    }

    fn reassemble<E: Packet>(&mut self, packet: E) -> Result<Option<E>, FragmentProcessError> {
        let parts = packet.into_parts();
        let id = parts.id();
        let correlation = parts.correlation_id();
        let payload = parts.payload();

        match decode_fragment_payload(&payload) {
            Ok(Some((header, fragment_payload))) => {
                match self.reassembler.push(header, fragment_payload) {
                    Ok(Some(message)) => {
                        let rebuilt = PacketParts::new(id, correlation, message.into_payload());
                        Ok(Some(E::from_parts(rebuilt)))
                    }
                    Ok(None) => Ok(None),
                    Err(err) => Err(FragmentProcessError::Reassembly(err)),
                }
            }
            Ok(None) => Ok(Some(E::from_parts(PacketParts::new(
                id,
                correlation,
                payload,
            )))),
            Err(err) => Err(FragmentProcessError::Decode(err)),
        }
    }

    fn purge_expired(&mut self) -> Vec<MessageId> { self.reassembler.purge_expired() }
}

impl<S, C, E> WireframeApp<S, C, E>
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
            .max_frame_length(self.buffer_capacity)
            .new_codec()
    }

    fn fragmentation_config(&self) -> Option<FragmentationConfig> {
        self.fragmentation
            .or_else(|| default_fragmentation(self.buffer_capacity))
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
        let mut codec = self.length_codec();
        let mut framed = BytesMut::with_capacity(bytes.len() + 4);
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
            warn!(
                "connection terminated with error: correlation_id={:?}, error={e:?}",
                None::<u64>
            );
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
        let codec = self.length_codec();
        let mut framed = Framed::new(stream, codec);
        framed.read_buffer_mut().reserve(self.buffer_capacity);
        let mut deser_failures = 0u32;
        let mut fragmentation = self.fragmentation_config().map(FragmentationState::new);
        let timeout_dur = Duration::from_millis(self.read_timeout_ms);

        loop {
            match timeout(timeout_dur, framed.next()).await {
                Ok(Some(Ok(buf))) => {
                    self.handle_frame(
                        &mut framed,
                        buf.as_ref(),
                        &mut deser_failures,
                        routes,
                        &mut fragmentation,
                    )
                    .await?;
                }
                Ok(Some(Err(e))) => return Err(e),
                Ok(None) => break,
                Err(_) => {
                    debug!("read timeout elapsed; continuing to wait for next frame");
                    if let Some(state) = fragmentation.as_mut() {
                        state.purge_expired();
                    }
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
        fragmentation: &mut Option<FragmentationState>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
        let Some(env) = self.decode_envelope(frame, deser_failures)? else {
            return Ok(());
        };
        let Some(env) = Self::reassemble_if_needed(fragmentation, deser_failures, env) else {
            return Ok(());
        };

        if let Some(service) = routes.get(&env.id) {
            self.forward_response(env, service, framed, fragmentation)
                .await?;
        } else {
            warn!(
                "no handler for message id: id={}, correlation_id={:?}",
                env.id, env.correlation_id
            );
        }

        Ok(())
    }

    fn decode_envelope(
        &self,
        frame: &[u8],
        deser_failures: &mut u32,
    ) -> Result<Option<Envelope>, io::Error> {
        match self.parse_envelope(frame) {
            Ok((env, _)) => {
                *deser_failures = 0;
                Ok(Some(env))
            }
            Err(EnvelopeDecodeError::Parse(e)) => {
                *deser_failures += 1;
                warn!(
                    "failed to parse message: correlation_id={:?}, error={e:?}",
                    None::<u64>
                );
                crate::metrics::inc_deser_errors();
                if *deser_failures >= MAX_DESER_FAILURES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many deserialization failures",
                    ));
                }
                Ok(None)
            }
            Err(EnvelopeDecodeError::Deserialize(e)) => {
                *deser_failures += 1;
                warn!(
                    "failed to deserialize message: correlation_id={:?}, error={e:?}",
                    None::<u64>
                );
                crate::metrics::inc_deser_errors();
                if *deser_failures >= MAX_DESER_FAILURES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many deserialization failures",
                    ));
                }
                Ok(None)
            }
        }
    }

    fn reassemble_if_needed(
        fragmentation: &mut Option<FragmentationState>,
        deser_failures: &mut u32,
        env: Envelope,
    ) -> Option<Envelope> {
        if let Some(state) = fragmentation.as_mut() {
            match state.reassemble(env) {
                Ok(Some(env)) => Some(env),
                Ok(None) => None,
                Err(FragmentProcessError::Decode(err)) => {
                    *deser_failures += 1;
                    warn!(
                        "failed to decode fragment header: correlation_id={:?}, error={err:?}",
                        None::<u64>
                    );
                    crate::metrics::inc_deser_errors();
                    None
                }
                Err(FragmentProcessError::Reassembly(err)) => {
                    *deser_failures += 1;
                    warn!(
                        "fragment reassembly failed: correlation_id={:?}, error={err:?}",
                        None::<u64>
                    );
                    crate::metrics::inc_deser_errors();
                    None
                }
            }
        } else {
            Some(env)
        }
    }

    async fn forward_response<W>(
        &self,
        env: Envelope,
        service: &HandlerService<E>,
        framed: &mut Framed<W, LengthDelimitedCodec>,
        fragmentation: &mut Option<FragmentationState>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        let request = ServiceRequest::new(env.payload, env.correlation_id);
        match service.call(request).await {
            Ok(resp) => {
                let parts = PacketParts::new(env.id, resp.correlation_id(), resp.into_inner())
                    .inherit_correlation(env.correlation_id);
                let correlation_id = parts.correlation_id();
                let responses = if let Some(state) = fragmentation.as_mut() {
                    match state.fragment(Envelope::from_parts(parts)) {
                        Ok(fragmented) => fragmented,
                        Err(err) => {
                            warn!(
                                "failed to fragment response: id={}, correlation_id={:?}, \
                                 error={err:?}",
                                env.id, correlation_id
                            );
                            crate::metrics::inc_handler_errors();
                            return Ok(());
                        }
                    }
                } else {
                    vec![Envelope::from_parts(parts)]
                };

                for response in responses {
                    match self.serializer.serialize(&response) {
                        Ok(bytes) => {
                            if let Err(e) = framed.send(bytes.into()).await {
                                warn!(
                                    "failed to send response: id={}, correlation_id={:?}, \
                                     error={e:?}",
                                    env.id, correlation_id
                                );
                                crate::metrics::inc_handler_errors();
                                break;
                            }
                        }
                        Err(e) => {
                            warn!(
                                "failed to serialize response: id={}, correlation_id={:?}, \
                                 error={e:?}",
                                env.id, correlation_id
                            );
                            crate::metrics::inc_handler_errors();
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    "handler error: id={}, correlation_id={:?}, error={e:?}",
                    env.id, env.correlation_id
                );
                crate::metrics::inc_handler_errors();
            }
        }

        Ok(())
    }
}
