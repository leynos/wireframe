# Parsing Frame Metadata

`FrameMetadata` allows a protocol to inspect frame headers before decoding the
entire payload. This can be useful for routing decisions when the message type
is encoded in a header field.

Implement the trait for your serializer or decoder that knows how to read the
header bytes. Only the minimal header portion should be read, returning the full
frame and the number of bytes consumed from the input.

```rust
use wireframe::frame::{FrameMetadata, FrameProcessor};
use wireframe::app::Envelope;
use bytes::BytesMut;

struct MyCodec;

impl FrameProcessor for MyCodec {
    type Frame = Vec<u8>;
    type Error = std::io::Error;

    fn decode(&self, src: &mut BytesMut) -> Result<Option<Self::Frame>, Self::Error> {
        todo!()
    }

    fn encode(&self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
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
