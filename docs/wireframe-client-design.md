# Client support in Wireframe

This document proposes an initial design for adding client-side protocol
support to `wireframe`. The goal is to reuse the existing framing,
serialization, and message abstractions while providing a small API for
connecting to a server and exchanging messages.

## Motivation

The library currently focuses on server development. However, the core layers
are intentionally generic: transport adapters, framing, serialization, routing,
and middleware form a pipeline that is largely independent of server-specific
logic. The design document outlines these layers, which process frames from raw
bytes to typed messages and back[^1]. Reusing these pieces enables the
implementation of a lightweight client without duplicating protocol code.

## Core components

### `WireframeClient`

A new `WireframeClient` type manages a single connection to a server. It
mirrors `WireframeServer` but operates in the opposite direction:

- Connect to a `TcpStream`, applying `SocketOptions` before the handshake.
- Optionally, send a preamble using the existing `Preamble` helpers.
- Encode outgoing messages using the selected `Serializer` and
  `tokio_util::codec::LengthDelimitedCodec` (4‑byte big‑endian prefix by
  default; configurable). Configure the codec’s `max_frame_length` on both the
  inbound (decode) and outbound (encode) paths to match the server’s frame
  capacity; otherwise, frames larger than the configured limit will fail.
- Decode incoming frames into typed responses.
- Expose async `send` and `receive` operations.

### Builder pattern

A `WireframeClient::builder()` method configures the client:

```rust
use std::net::SocketAddr;

use wireframe::{BincodeSerializer, WireframeClient};

let addr: SocketAddr = "127.0.0.1:7878".parse()?;
let client = WireframeClient::builder()
    .serializer(BincodeSerializer)
    .max_frame_length(1024)
    .connect(addr)
    .await?;
```

The same `Serializer` trait used by the server is reused here, ensuring
messages are encoded consistently while framing is handled by the
length‑delimited codec.

### Request/response helpers

To keep the API simple, `WireframeClient` offers a `call` method that sends a
message implementing `Message` and waits for the next response frame:

```rust
let request = Login { username: "guest".into() };
let response: LoginAck = client.call(&request).await?;
```

Internally, this uses the `Serializer` to encode the request, sends it through
the length‑delimited codec, then waits for a frame, decodes it, and
deserializes the response type.

### Implementation decisions

- `connect` accepts a `SocketAddr` so the client can create a `TcpSocket` and
  apply socket options before connecting.
- `ClientCodecConfig` captures the length prefix format and maximum frame
  length, clamping the frame length to match server bounds (64 bytes to 16 MiB).
- The default `max_frame_length` is 1024 bytes to mirror the server builder’s
  default buffer capacity.

### Connection lifecycle

Like the server, the client should expose hooks for connection setup and
teardown. These mirror the server’s lifecycle callbacks, so both sides can
share initialization logic.

## Example usage

```rust
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use std::net::SocketAddr;

    let mut client = WireframeClient::builder()
        .serializer(BincodeSerializer)
        .max_frame_length(1024)
        .connect("127.0.0.1:7878".parse::<SocketAddr>()?)
        .await?;

    let login = Login { username: "guest".into() };
    let ack: LoginAck = client.call(&login).await?;
    println!("logged in: {:?}", ack);
    Ok(())
}
```

## Future work

This initial design focuses on a basic request/response workflow. Future
extensions might include:

- Middleware support for outgoing and incoming frames.
- Connection pooling for protocols that open multiple simultaneous connections.
- Helper traits for streaming or multiplexed protocols.

By leveraging the existing abstractions for framing and serialization, client
support can share most of the server’s implementation while providing a small
ergonomic API.

[^1]: See
      [wireframe router design](rust-binary-router-library-design.md#implementation-details).
