//! Connection handling and response utilities for `WireframeApp`.

use std::collections::HashMap;

use bytes::BytesMut;
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{Duration, timeout},
};

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

/// Number of idle polls before terminating a connection.
const MAX_IDLE_POLLS: u32 = 50; // ~5s with 100ms timeout
/// Maximum consecutive deserialization failures before closing a connection.
const MAX_DESER_FAILURES: u32 = 10;

impl<S, C, E> WireframeApp<S, C, E>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
{
    /// Serialize `msg` and write it to `stream` using the frame processor.
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
        let mut framed = BytesMut::with_capacity(4 + bytes.len());
        self.frame_processor
            .encode(&bytes, &mut framed)
            .map_err(SendError::Io)?;
        stream.write_all(&framed).await.map_err(SendError::Io)?;
        stream.flush().await.map_err(SendError::Io)
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
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            .or_else(|_| self.serializer.deserialize::<Envelope>(frame))
    }

    /// Handle an accepted connection.
    ///
    /// This placeholder immediately closes the connection after the
    /// preamble phase. A warning is logged so tests and callers are
    /// aware of the current limitation.
    pub async fn handle_connection<W>(&self, mut stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let state = if let Some(setup) = &self.on_connect {
            Some((setup)().await)
        } else {
            None
        };

        let routes = self.build_chains().await;

        if let Err(e) = self.process_stream(&mut stream, &routes).await {
            tracing::warn!(correlation_id = ?None::<u64>, error = ?e, "connection terminated with error");
        }

        if let (Some(teardown), Some(state)) = (&self.on_disconnect, state) {
            teardown(state).await;
        }
    }

    async fn build_chains(&self) -> HashMap<u32, HandlerService<E>> {
        let mut routes = HashMap::new();
        for (&id, handler) in &self.routes {
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
        stream: &mut W,
        routes: &HashMap<u32, HandlerService<E>>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        let mut buf = BytesMut::with_capacity(1024);
        let mut idle = 0u32;
        let mut deser_failures = 0u32;

        loop {
            if let Some(frame) = self.frame_processor.decode(&mut buf)? {
                self.handle_frame(stream, &frame, &mut deser_failures, routes)
                    .await?;
                idle = 0;
                continue;
            }

            if self.read_and_update(stream, &mut buf, &mut idle).await? {
                break;
            }
        }

        Ok(())
    }

    async fn read_and_update<W>(
        &self,
        stream: &mut W,
        buf: &mut BytesMut,
        idle: &mut u32,
    ) -> io::Result<bool>
    where
        W: AsyncRead + AsyncWrite + Unpin,
    {
        match self.read_into(stream, buf).await {
            Ok(Some(0)) => Ok(true),
            Ok(Some(_)) => {
                *idle = 0;
                Ok(false)
            }
            Ok(None) => {
                *idle += 1;
                Ok(*idle >= MAX_IDLE_POLLS)
            }
            Err(e) if Self::is_transient_error(&e) => Ok(false),
            Err(e) if Self::is_fatal_error(&e) => Ok(true),
            Err(e) => Err(e),
        }
    }

    fn is_transient_error(e: &io::Error) -> bool {
        matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        )
    }

    fn is_fatal_error(e: &io::Error) -> bool {
        matches!(
            e.kind(),
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
        )
    }

    async fn read_into<W>(&self, stream: &mut W, buf: &mut BytesMut) -> io::Result<Option<usize>>
    where
        W: AsyncRead + Unpin,
    {
        match timeout(Duration::from_millis(100), stream.read_buf(buf)).await {
            Ok(Ok(n)) => Ok(Some(n)),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(None),
        }
    }

    async fn handle_frame<W>(
        &self,
        stream: &mut W,
        frame: &[u8],
        deser_failures: &mut u32,
        routes: &HashMap<u32, HandlerService<E>>,
    ) -> io::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
        let (env, _) = match self.parse_envelope(frame) {
            Ok(result) => {
                *deser_failures = 0;
                result
            }
            Err(e) => {
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
                    if let Err(e) = self.send_response(stream, &response).await {
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
                    tracing::warn!(id = env.id, correlation_id = ?env.correlation_id, error = ?e, "handler error");
                    crate::metrics::inc_handler_errors();
                }
            }
        } else {
            tracing::warn!(id = env.id, correlation_id = ?env.correlation_id, "no handler for message id");
            crate::metrics::inc_handler_errors();
        }

        Ok(())
    }
}
