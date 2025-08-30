# Parsing frame metadata

`FrameMetadata` allows a protocol to inspect frame headers before decoding the
entire payload. This can be useful for routing decisions when the message type
is encoded in a header field.

Implement the trait for your serialiser or decoder that knows how to read the
header bytes. Only the minimal header portion should be read, returning the
frame envelope and the number of bytes consumed from the input.

```rust
use wireframe::frame::FrameMetadata;
use wireframe::app::Envelope;
use tokio_util::codec::{Decoder, Encoder};
use bytes::{Bytes, BytesMut};

struct MyCodec;
// Example only: implement the minimal Decoder/Encoder surface for docs.
impl Decoder for MyCodec {
    // Using BytesMut here mirrors LengthDelimitedCodec’s default Item.
    type Item = BytesMut;
    type Error = std::io::Error;

    fn decode(
        &mut self,
        _src: &mut BytesMut,
    ) -> Result<Option<Self::Item>, Self::Error> {
        todo!()
    }
}

impl Encoder<Bytes> for MyCodec {
    type Error = std::io::Error;

    fn encode(&mut self, _item: Bytes, _dst: &mut BytesMut) -> Result<(), Self::Error> {
        todo!()
    }
}

impl FrameMetadata for MyCodec {
    type Frame = Envelope;
    type Error = std::io::Error;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        // read header fields here and return the full frame
        todo!()
    }
}
```

In `WireframeApp` the metadata parser is used before deserialisation so routes
can be selected as soon as the header is available.
