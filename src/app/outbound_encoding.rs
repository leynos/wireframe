//! Shared outbound encoding helpers for application responses.
//!
//! The helpers in this module keep serialization and codec frame wrapping in
//! one place while leaving raw-stream and framed-transport writes at the edge.
//!
//! TODO: Track the zero-copy serializer output migration in
//! <https://github.com/leynos/wireframe/issues/538>. The current serializer
//! contract returns `Vec<u8>`, so this module keeps the `Bytes::from`
//! conversion until that public contract can change deliberately.

use bytes::Bytes;

use super::SendError;
use crate::{codec::FrameCodec, message::EncodeWith, serializer::Serializer};

/// A serialized message wrapped into a transport codec frame.
#[derive(Debug)]
pub(crate) struct EncodedFrame<T> {
    pub(crate) frame: T,
}

/// Serialize `msg` and wrap it with `codec`.
pub(crate) fn encode_message_frame<S, F, M>(
    serializer: &S,
    codec: &F,
    msg: &M,
) -> Result<EncodedFrame<F::Frame>, SendError>
where
    S: Serializer + Send + Sync,
    F: FrameCodec,
    M: EncodeWith<S>,
{
    let bytes = serializer.serialize(msg).map_err(SendError::Serialize)?;
    // Keep this bridge behaviour-preserving until issue #538 changes the
    // public serializer contract to return a Bytes-native container.
    let frame = codec.wrap_payload(Bytes::from(bytes));
    Ok(EncodedFrame { frame })
}

#[cfg(test)]
mod tests {
    //! Tests for app outbound encoding helper behaviour.

    use std::{error::Error, io};

    use bytes::{Bytes, BytesMut};
    use googletest::prelude::*;
    use rstest::rstest;
    use tokio_util::codec::{Decoder, Encoder};

    use super::*;
    use crate::{message::DecodeWith, serializer::Serializer};

    #[derive(Clone, Debug)]
    struct VisibleCodec {
        tag: &'static str,
    }

    #[derive(Debug, PartialEq)]
    struct VisibleFrame {
        tag: &'static str,
        payload: Bytes,
    }

    struct NoopDecoder;

    impl Decoder for NoopDecoder {
        type Error = io::Error;
        type Item = VisibleFrame;

        fn decode(&mut self, _src: &mut BytesMut) -> io::Result<Option<Self::Item>> { Ok(None) }
    }

    struct NoopEncoder;

    impl Encoder<VisibleFrame> for NoopEncoder {
        type Error = io::Error;

        fn encode(&mut self, _item: VisibleFrame, _dst: &mut BytesMut) -> io::Result<()> { Ok(()) }
    }

    impl FrameCodec for VisibleCodec {
        type Decoder = NoopDecoder;
        type Encoder = NoopEncoder;
        type Frame = VisibleFrame;

        fn decoder(&self) -> Self::Decoder { NoopDecoder }

        fn encoder(&self) -> Self::Encoder { NoopEncoder }

        fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_ref() }

        fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.payload.clone() }

        fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
            VisibleFrame {
                tag: self.tag,
                payload,
            }
        }

        fn max_frame_length(&self) -> usize { 1024 }
    }

    #[derive(Debug)]
    struct TestMessage;

    #[derive(Clone, Copy)]
    struct TestSerializer {
        should_fail: bool,
    }

    impl EncodeWith<TestSerializer> for TestMessage {
        fn encode_with(
            &self,
            serializer: &TestSerializer,
        ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
            if serializer.should_fail {
                Err(Box::new(io::Error::other("serialize failed")))
            } else {
                Ok(vec![1, 2, 3, 4])
            }
        }
    }

    impl DecodeWith<TestSerializer> for TestMessage {
        fn decode_with(
            _serializer: &TestSerializer,
            _bytes: &[u8],
            _context: &crate::message::DeserializeContext<'_>,
        ) -> Result<(Self, usize), Box<dyn Error + Send + Sync>> {
            Ok((Self, 0))
        }
    }

    impl Serializer for TestSerializer {
        fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
        where
            M: EncodeWith<Self>,
        {
            value.encode_with(self)
        }

        fn deserialize<M>(&self, _bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
        where
            M: DecodeWith<Self>,
        {
            Err(Box::new(io::Error::other("deserialize unused")))
        }
    }

    #[rstest]
    #[case("binary")]
    #[case("text")]
    fn encode_message_frame_wraps_serializer_output_with_codec(#[case] tag: &'static str) {
        let encoded = encode_message_frame(
            &TestSerializer { should_fail: false },
            &VisibleCodec { tag },
            &TestMessage,
        )
        .expect("encoding should succeed");

        assert_that!(
            &encoded.frame,
            eq(&VisibleFrame {
                tag,
                payload: Bytes::from_static(&[1, 2, 3, 4]),
            })
        );
    }

    #[test]
    fn encode_message_frame_propagates_serialization_failure() {
        let result = encode_message_frame(
            &TestSerializer { should_fail: true },
            &VisibleCodec { tag: "binary" },
            &TestMessage,
        );
        assert_that!(result, err(pat!(SendError::Serialize(_))));
    }
}
