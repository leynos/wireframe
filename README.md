# Wireframe

**Wireframe** is an experimental Rust library that simplifies building servers
and clients for custom binary protocols. The design borrows heavily from
[Actix Web](https://actix.rs/) to provide a familiar, declarative API for
routing, extractors, and middleware.

## Motivation

Manual handling of binary protocols typically involves verbose serialization
code, custom frame parsing, and complex dispatch logic. `wireframe` aims to
reduce this boilerplate through layered abstractions:

- **Transport adapter** built on Tokio I/O
- **Framing layer** for length‑prefixed or custom frames
- **Connection preamble** with customizable validation callbacks
  \[[docs](docs/preamble-validator.md)\]
- Call `with_preamble::<T>()` before registering success or failure callbacks
- **Serialization engine** using `bincode` or a `wire-rs` wrapper
- **Routing engine** that dispatches messages by ID
- **Handler invocation** with extractor support
- **Middleware chain** for request/response processing
- **[Connection lifecycle hooks](#connection-lifecycle)** for per-connection
  setup and teardown

These layers correspond to the architecture outlined in the design
document【F:docs/rust-binary-router-library-design.md†L292-L344】.

## API Overview

Applications are configured using a builder pattern similar to Actix Web. A
`WireframeApp` defines routes and middleware, while `WireframeServer` manages
connections and runs the Tokio event loop:

```rust
WireframeServer::new(|| {
    WireframeApp::new()
        .frame_processor(MyFrameProcessor::new())
        .app_data(state.clone())
        .route(MessageType::Login, handle_login)
        .wrap(MyLoggingMiddleware::default())
})
.bind("127.0.0.1:7878")?
.run()
.await
```

By default, the number of worker tasks equals the number of CPU cores. If the
CPU count cannot be determined, the server falls back to a single worker.

The builder supports methods like `frame_processor`, `route`, `app_data`, and
`wrap` for middleware configuration. `app_data` stores any `Send + Sync` value
keyed by type; registering another value of the same type overwrites the
previous one. Handlers retrieve these values using the `SharedState<T>`
extractor【F:docs/rust-binary-router-library-design.md†L622-L710】.

Handlers are asynchronous functions whose parameters implement extractor traits
and may return responses implementing the `Responder` trait. This pattern
mirrors Actix Web handlers and keeps protocol logic
concise【F:docs/rust-binary-router-library-design.md†L682-L710】.

## Example

The design document includes a simple echo server that demonstrates routing
based on a message ID and the use of a length‑prefixed frame processor:

```rust
async fn handle_echo(req: Message<EchoRequest>) -> WireframeResult<EchoResponse> {
    Ok(EchoResponse {
        original_payload: req.payload.clone(),
        echoed_at: time_now(),
    })
}

WireframeServer::new(|| {
    WireframeApp::new()
        .serializer(BincodeSerializer)
        .route(MyMessageType::Echo, handle_echo)
})
.bind("127.0.0.1:8000")?
.run()
.await
```

This example showcases how derive macros and the framing abstraction simplify a
binary protocol server【F:docs/rust-binary-router-library-design.md†L1126-L1156】.

## Custom Envelopes

`WireframeApp` defaults to a simple `Envelope` containing a message ID and raw
payload bytes. Applications can supply their own envelope type by calling
`WireframeApp::<_, _, MyEnv>::new_with_envelope()`. The custom type must
implement the `Packet` trait:

```rust
use wireframe::app::{Packet, WireframeApp};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct MyEnv { id: u32, data: Vec<u8> }

impl Packet for MyEnv {
    fn id(&self) -> u32 { self.id }
    fn into_parts(self) -> (u32, Vec<u8>) { (self.id, self.data) }
    fn from_parts(id: u32, data: Vec<u8>) -> Self { Self { id, data } }
}

let app = WireframeApp::<_, _, MyEnv>::new_with_envelope()
    .unwrap()
    .route(1, std::sync::Arc::new(|env: &MyEnv| Box::pin(async move { /* ... */ })))
    .unwrap();
```

This allows integration with existing packet formats without modifying
`handle_frame`.

## Response Serialization and Framing

Handlers can return types implementing the `Responder` trait. These values are
encoded using the application's configured serializer and written back through
the `FrameProcessor`【F:docs/rust-binary-router-library-design.md†L724-L730】.

The included `LengthPrefixedProcessor` illustrates a simple framing strategy
that prefixes each frame with its length. The format is configurable (prefix
size and endianness) and defaults to a 4‑byte big‑endian length
prefix【F:docs/rust-binary-router-library-design.md†L1082-L1123】.

```rust
use wireframe::frame::{LengthFormat, LengthPrefixedProcessor};

let app = WireframeApp::new()?
    .frame_processor(LengthPrefixedProcessor::new(LengthFormat::u16_le()));
```

## Connection Lifecycle

`WireframeApp` can run callbacks when a connection is opened or closed. The
state produced by `on_connection_setup` is passed to `on_connection_teardown`
when the connection ends.

```rust
    let app = WireframeApp::new()
        .on_connection_setup(|| async { 42u32 })
        .on_connection_teardown(|state| async move {
            println!("closing with {state}");
        });
```

## Session Registry

The \[`SessionRegistry`\] stores weak references to \[`PushHandle`\]s for active
connections. Background tasks can look up a handle by \[`ConnectionId`\] to send
frames asynchronously without keeping the connection alive.

```rust
use wireframe::{
    session::{ConnectionId, SessionRegistry},
    push::PushHandle,
    ConnectionContext,
};

let registry: SessionRegistry<MyFrame> = SessionRegistry::default();

// inside a `WireframeProtocol` implementation
fn on_connection_setup(&self, handle: PushHandle<MyFrame>, _ctx: &mut ConnectionContext) {
    let id = ConnectionId::from(42);
    registry.insert(id, &handle);
}
```

## Custom Extractors

Extractors are types that implement `FromMessageRequest`. When a handler lists
an extractor as a parameter, `wireframe` automatically constructs it using the
incoming \[`MessageRequest`\] and remaining \[`Payload`\]. Built‑in extractors
like `Message<T>`, `SharedState<T>` and `ConnectionInfo` decode the payload,
access app state or expose peer information.

Custom extractors let you centralize parsing and validation logic that would
otherwise be duplicated across handlers. A session token parser, for example,
can verify the token before any route-specific code executes
[Design Guide: Data Extraction and Type Safety][data-extraction-guide].

```rust
use wireframe::extractor::{ConnectionInfo, FromMessageRequest, MessageRequest, Payload};

pub struct SessionToken(String);

impl FromMessageRequest for SessionToken {
    type Error = std::convert::Infallible;

    fn from_message_request(
        _req: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        let len = payload.data[0] as usize;
        let token = std::str::from_utf8(&payload.data[1..=len]).unwrap().to_string();
        payload.advance(1 + len);
        Ok(Self(token))
    }
}
```

Custom extractors integrate seamlessly with other parameters:

```rust
async fn handle_ping(token: SessionToken, info: ConnectionInfo) {
    println!("{} from {:?}", token.0, info.peer_addr());
}
```

## Middleware

Middleware allows inspecting or modifying requests and responses. The `from_fn`
helper builds middleware from an async function or closure:

```rust
use wireframe::middleware::from_fn;

let logging = from_fn(|req, next| async move {
    tracing::info!("received request: {:?}", req);
    let res = next.call(req).await?;
    tracing::info!("sending response: {:?}", res);
    Ok(res)
});
```

## Examples

Example programs are available in the `examples/` directory:

- `echo.rs` — minimal echo server using routing
- `ping_pong.rs` — showcases serialization and middleware in a ping/pong
  protocol. See [examples/ping_pong.md](examples/ping_pong.md) for a detailed
  overview.
- `packet_enum.rs` – shows packet type discrimination with a bincode enum and a
  frame containing container types like `HashMap` and `Vec`.

Run an example with Cargo:

```bash
cargo run --example echo
```

Try the echo server with netcat:

```bash
$ cargo run --example echo
# in another terminal
$ printf '\x00\x00\x00\x00\x01\x00\x00\x00' | nc 127.0.0.1 7878 | xxd
```

Try the ping‑pong server with netcat:

```bash
$ cargo run --example ping_pong
# in another terminal
$ printf '\x00\x00\x00\x08\x01\x00\x00\x00\x2a\x00\x00\x00' | nc 127.0.0.1 7878 | xxd
```

## Current Limitations

Connection handling now processes frames and routes messages. Although the
server is still experimental, it now compiles in release mode for evaluation or
production use.

## Roadmap

Development priorities are tracked in [docs/roadmap.md](docs/roadmap.md). Key
tasks include building the Actix‑inspired API, implementing middleware and
extractor traits, and providing example applications【F:docs/roadmap.md†L1-L24】.

## License

Wireframe is distributed under the terms of the ISC license. See
[LICENSE](LICENSE) for details.

[data-extraction-guide]: docs/rust-binary-router-library-design.md#53-data-extraction-and-type-safety
