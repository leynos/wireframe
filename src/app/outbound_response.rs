//! Public response-sending helpers for `WireframeApp`.
//!
//! This module keeps outbound response serialization and write helpers out of
//! the inbound frame-dispatch path while preserving the public API.

use bytes::BytesMut;
use futures::SinkExt;
use tokio::io::{self, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{Encoder, Framed, LengthDelimitedCodec};

use super::{
    builder::WireframeApp,
    envelope::Packet,
    error::SendError,
    outbound_encoding::encode_message_frame,
};
use crate::{
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    message::EncodeWith,
    serializer::Serializer,
};

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
        M: EncodeWith<S>,
    {
        let outbound_frame = encode_message_frame(&self.serializer, &self.codec, msg)?;
        let mut encoder = self.codec.encoder();
        let mut encoded_buf = BytesMut::new();
        encoder
            .encode(outbound_frame.frame, &mut encoded_buf)
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
        W: AsyncWrite + Unpin,
        M: EncodeWith<S>,
        Cc: Encoder<F::Frame, Error = io::Error>,
    {
        let encoded = encode_message_frame(&self.serializer, &self.codec, msg)?;
        framed.send(encoded.frame).await.map_err(SendError::Io)
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
        W: AsyncWrite + Unpin,
        M: EncodeWith<S>,
    {
        let bytes = self
            .serializer
            .serialize(msg)
            .map_err(SendError::Serialize)?;
        // `LengthDelimitedCodec` performs the length-prefix encoding in
        // `framed.send`. Do not use `encode_message_frame` here: wrapping the
        // payload with `self.codec` would add a second application frame around
        // a path that intentionally sends the raw serialized message.
        // TODO: Remove this `Vec<u8>` to `Bytes` conversion when the
        // zero-copy serializer migration in
        // https://github.com/leynos/wireframe/issues/538 is complete.
        framed.send(bytes.into()).await.map_err(SendError::Io)
    }
}
