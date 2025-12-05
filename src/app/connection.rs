//! Connection handling and response utilities for `WireframeApp`.

use std::{collections::HashMap, sync::Arc};

use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use log::{debug, warn};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, AsyncWriteExt},
    time::{Duration, timeout},
};
use tokio_util::codec::{Encoder, Framed, LengthDelimitedCodec};

use super::{
    builder::{WireframeApp, default_fragmentation},
    envelope::{Envelope, Packet},
    error::SendError,
    fragmentation_state::FragmentationState,
    frame_handling,
};
use crate::{
    fragment::FragmentationConfig,
    frame::FrameMetadata,
    message::Message,
    middleware::HandlerService,
    serializer::Serializer,
};

fn purge_expired(fragmentation: &mut Option<FragmentationState>) {
    if let Some(frag) = fragmentation.as_mut() {
        frag.purge_expired();
    }
}

/// Maximum consecutive deserialization failures before closing a connection.
const MAX_DESER_FAILURES: u32 = 10;

/// Per-frame processing state bundled for `handle_frame`.
struct FrameHandlingContext<'a, E, W>
where
    E: Packet,
    W: AsyncRead + AsyncWrite + Unpin,
{
    framed: &'a mut Framed<W, LengthDelimitedCodec>,
    deser_failures: &'a mut u32,
    routes: &'a HashMap<u32, HandlerService<E>>,
    fragmentation: &'a mut Option<FragmentationState>,
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
    ) -> std::result::Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>> {
        self.serializer
            .parse(frame)
            .map_err(Box::<dyn std::error::Error + Send + Sync>::from)
            .or_else(|_| self.serializer.deserialize::<Envelope>(frame))
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
                        buf.as_ref(),
                        FrameHandlingContext {
                            framed: &mut framed,
                            deser_failures: &mut deser_failures,
                            routes,
                            fragmentation: &mut fragmentation,
                        },
                    )
                    .await?;
                }
                Ok(Some(Err(e))) => return Err(e),
                Ok(None) => break,
                Err(_) => {
                    debug!("read timeout elapsed; continuing to wait for next frame");
                    purge_expired(&mut fragmentation);
                }
            }
        }

        Ok(())
    }

    async fn handle_frame<W>(
        &self,
        frame: &[u8],
        ctx: FrameHandlingContext<'_, E, W>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        let FrameHandlingContext {
            framed,
            deser_failures,
            routes,
            fragmentation,
        } = ctx;

        crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
        let Some(env) = self.decode_envelope(frame, deser_failures)? else {
            return Ok(());
        };
        let Some(env) = frame_handling::reassemble_if_needed(
            fragmentation,
            deser_failures,
            env,
            MAX_DESER_FAILURES,
        )?
        else {
            return Ok(());
        };

        if let Some(service) = routes.get(&env.id) {
            frame_handling::forward_response(
                env,
                service,
                frame_handling::ResponseContext {
                    serializer: &self.serializer,
                    framed,
                    fragmentation,
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

    /// Increment deserialization failures and close the connection if the threshold is exceeded.
    fn handle_decode_failure(
        deser_failures: &mut u32,
        context: &str,
        err: impl std::fmt::Debug,
    ) -> Result<Option<Envelope>, io::Error> {
        *deser_failures += 1;
        warn!("{context}: correlation_id={:?}, error={err:?}", None::<u64>);
        crate::metrics::inc_deser_errors();
        if *deser_failures >= MAX_DESER_FAILURES {
            warn!("closing connection after {deser_failures} deserialization failures: {context}");
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many deserialization failures",
            ));
        }
        Ok(None)
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
            Err(err) => {
                Self::handle_decode_failure(deser_failures, "failed to decode message", err)
            }
        }
    }
}
