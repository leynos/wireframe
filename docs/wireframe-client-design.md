# Client support in Wireframe

This document proposes an initial design for adding client-side protocol
support to `wireframe`. The goal is to reuse the existing framing,
serialization, and message abstractions while providing a small API for
connecting to a server and exchanging messages.

## Motivation

The library currently focuses on server development. However, the core layers
are intentionally generic: transport adaptors, framing, serialization, routing,
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

#### Request/response error mapping

Client request/response failures are mapped to `WireframeError` variants, so
client and server semantics stay aligned:

- Transport failures (socket closure, read/write failures) surface as
  `ClientError::Wireframe(WireframeError::Io(_))`.
- Decode failures after a frame is received surface as
  `ClientError::Wireframe(WireframeError::Protocol( ClientProtocolError::Deserialize(_)))`.

This mapping applies to `receive`, `call`, and envelope-aware receive/call
methods. Serialization failures remain explicit as `ClientError::Serialize(_)`
because they occur before the transport layer is used.

### Implementation decisions

- `connect` accepts a `SocketAddr` so the client can create a `TcpSocket` and
  apply socket options before connecting.
- `ClientCodecConfig` captures the length prefix format and maximum frame
  length, clamping the frame length to match server bounds (64 bytes to 16 MiB).
- The default `max_frame_length` is 1024 bytes to mirror the server builder’s
  default buffer capacity.

### Connection lifecycle

The client builder exposes lifecycle hooks that mirror the server's hook
system, enabling consistent instrumentation across client and server:

- **Setup hook** (`on_connection_setup`): Runs after TCP connection and
  preamble exchange (if configured) succeed. Returns connection-specific state
  of type `C` stored in the client for the connection's lifetime. The setup
  callback must be an `FnOnce` closure returning a `Future` that produces `C`.

- **Teardown hook** (`on_connection_teardown`): Runs when `close()` is called.
  Receives the state `C` produced by the setup hook. The teardown hook only
  executes if a setup hook was configured and successfully produced state.

- **Error hook** (`on_error`): Runs when errors occur during `send`, `receive`,
  or `call` operations. Receives a reference to the `ClientError` for logging
  or metrics. This hook is independent of the setup/teardown pair.

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

struct SessionState {
    request_count: AtomicUsize,
}

let client = WireframeClient::builder()
    .on_connection_setup(|| async {
        SessionState { request_count: AtomicUsize::new(0) }
    })
    .on_connection_teardown(|state: SessionState| async move {
        println!("Session ended after {} requests",
            state.request_count.load(Ordering::SeqCst));
    })
    .on_error(|err| async move {
        eprintln!("Client error: {err}");
    })
    .connect(addr)
    .await?;

// Use the client...
client.close().await; // Teardown hook invoked here
```

The builder uses a type-changing pattern for `on_connection_setup`: calling it
changes the `C` type parameter from the default `()` to the user's state type.
This ensures type safety—teardown callbacks must accept the exact state type
produced by setup.

### Preamble support

The client builder now supports an optional preamble exchange. Use
`with_preamble` to send a typed preamble after TCP connect completes:

```rust
use std::time::Duration;
use futures::FutureExt;

#[derive(bincode::Encode)]
struct ClientHello { version: u16 }

let client = WireframeClient::builder()
    .with_preamble(ClientHello { version: 1 })
    .preamble_timeout(Duration::from_secs(5))
    .on_preamble_success(|_preamble, stream| {
        async move {
            // Read server response, return leftover bytes
            Ok(Vec::new())
        }
        .boxed()
    })
    .on_preamble_failure(|err, _stream| {
        async move {
            eprintln!("Preamble failed: {err}");
            Ok(())
        }
        .boxed()
    })
    .connect(addr)
    .await?;
```

The success callback runs after the preamble is written successfully and can
read the server's response (using `read_preamble` from the preamble module).
Any bytes read beyond the server's response must be returned as "leftover"
bytes for the framing layer to replay. The failure callback runs when the
preamble exchange fails due to timeout, I/O error, or encoding error.

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
