# Client Support in Wireframe

This document proposes an initial design for adding client-side protocol
support to `wireframe`. The goal is to reuse the existing framing,
serialization, and message abstractions while providing a small API for
connecting to a server and exchanging messages.

## Motivation

The library currently focuses on server development. However, the core layers
are intentionally generic: transport adapters, framing, serialization, routing,
and middleware form a pipeline that is largely independent of server-specific
logic. The design document outlines these layers, which process frames from raw
bytes to typed messages and back[^router-design]. Reusing these pieces enables
the implementation of a lightweight client without duplicating protocol code.

## Core Components

### `WireframeClient`

A new `WireframeClient` type manages a single connection to a server. It
mirrors `WireframeServer` but operates in the opposite direction:

- Connect to a `TcpStream`.
- Optionally, send a preamble using the existing `Preamble` helpers.
- Encode outgoing messages using the selected `Serializer` and `FrameProcessor`.
- Decode incoming frames into typed responses.
- Expose async `send` and `receive` operations.

### Builder Pattern

A `WireframeClient::builder()` method configures the client:

```rust
let client = WireframeClient::builder()
    .frame_processor(LengthPrefixedProcessor::new(LengthFormat::u32_be()))
    .serializer(BincodeSerializer)
    .connect("127.0.0.1:7878")
    .await?;
```

The same `FrameProcessor` and `Serializer` traits used by the server are reused
here, ensuring messages are framed and encoded consistently.

### Request/Response Helpers

To keep the API simple, `WireframeClient` offers a `call` method that sends a
message implementing `Message` and waits for the next response frame:

```rust
let request = Login { username: "guest".into() };
let response: LoginAck = client.call(request).await?;
```

Internally, this uses the `Serializer` to encode the request, writes it through
the `FrameProcessor`, then waits for a frame, decodes it, and deserializes the
response type.

### Connection Lifecycle

Like the server, the client should expose hooks for connection setup and
teardown. These mirror the server’s lifecycle callbacks so both sides can share
initialization logic.

## Example Usage

```rust
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut client = WireframeClient::builder()
        .frame_processor(LengthPrefixedProcessor::new(LengthFormat::u32_be()))
        .serializer(BincodeSerializer)
        .connect("127.0.0.1:7878")
        .await?;

    let login = Login { username: "guest".into() };
    let ack: LoginAck = client.call(login).await?;
    println!("logged in: {:?}", ack);
    Ok(())
}
```

## Future Work

This initial design focuses on a basic request/response workflow. Future
extensions might include:

- Middleware support for outgoing and incoming frames.
- Connection pooling for protocols that open multiple simultaneous connections.
- Helper traits for streaming or multiplexed protocols.

By leveraging the existing abstractions for framing and serialization, client
support can share most of the server’s implementation while providing a small
ergonomic API.
[^router-design]: See [wireframe router
                  design](rust-binary-router-library-design.md#implementation-details).
